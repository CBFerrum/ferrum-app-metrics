//! Metrics. A metric is a Studio query of exactly one measure, checked against the records and the
//! author's rights when it is saved. Its lineage is the record and the fields it reads. A Data
//! Steward who does not own it certifies it, which fixes its definition: changing the definition
//! puts it back in draft. Whoever reads it gets its value with their own rights to the records it
//! counts, so two people may read two numbers from one definition. A deprecated metric names what
//! replaces it, and is read no more. Datasets (`dataset`) are governed the same way, by the same
//! helpers here.

use crate::obj;
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use ferrum_calc::formula::Formula;
use ferrum_domain::util::{elevated, id, s, sys};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use ferrum_studio::spec::{Agg, Bucket, GroupBy, MAX_LIMIT, Spec, TimeSeries};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

/// A record's definition, as a Studio query.
pub(crate) fn parse_spec(doc: &Doc) -> Result<Spec> {
    let raw = doc.get("definition").cloned().unwrap_or(Value::Null);
    let raw = match raw {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| Error::field("definition", format!("is not a Studio query: {e}")))?,
        v => v,
    };
    serde_json::from_value(raw).map_err(|e| Error::field("definition", format!("is not a Studio query: {e}")))
}

/// A metric's definition, as a Studio query of one measure.
pub(crate) fn spec_of(doc: &Doc) -> Result<Spec> {
    let spec = parse_spec(doc)?;
    if spec.measures.len() != 1 || !spec.group_by.is_empty() || spec.pivot.is_some() || spec.time_series.is_some() {
        return Err(Error::field("definition", "a metric is one measure, ungrouped"));
    }
    Ok(spec)
}

/// How deep metrics may be composed of others.
const MAX_DEPTH: usize = 8;
/// Most metrics one formula may name.
const MAX_PARTS: usize = 20;

/// What a metric is: a Studio query of one measure (summing amounts in several currencies in one when
/// it converts them), or a formula over other metrics by name (`[revenue] / [orders]`), worked out in
/// exact decimals from each part's value for the reader.
pub(crate) enum Kind {
    Query(Box<Spec>, Option<Conversion>),
    Composed { text: String, formula: Formula, parts: Vec<String> },
}

/// The metrics a formula names, each once, in order of name.
pub(crate) fn parts_of(text: &str) -> Result<Vec<String>> {
    let mut parts = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let close = rest[open..].find(']').ok_or_else(|| Error::field("formula", "a [ is never closed"))?;
        let name = rest[open + 1..open + close].trim();
        if name.is_empty() {
            return Err(Error::field("formula", "[] names no metric"));
        }
        parts.insert(name.to_string());
        rest = &rest[open + close + 1..];
    }
    if parts.is_empty() {
        return Err(Error::field("formula", "names no metric: write each in brackets, [revenue] / [orders]"));
    }
    if parts.len() > MAX_PARTS {
        return Err(Error::field("formula", format!("names at most {MAX_PARTS} metrics")));
    }
    Ok(parts.into_iter().collect())
}

pub(crate) fn kind_of(doc: &Doc) -> Result<Kind> {
    let formula = s(doc, "formula").map(str::trim).filter(|f| !f.is_empty());
    let defined = !matches!(doc.get("definition"), None | Some(Value::Null)) && doc.get("definition") != Some(&json!(""));
    match (formula, defined) {
        (Some(_), true) => Err(Error::field("formula", "a metric is a query or a formula over other metrics, not both")),
        (Some(text), false) => {
            if conversion_of(doc)?.is_some() {
                return Err(Error::field("convert_to", "a composed metric is converted by its parts, each its own"));
            }
            let parts = parts_of(text)?;
            let formula = Formula::parse(text, &|name| Ok(name.trim().to_string())).map_err(|e| Error::field("formula", e))?;
            Ok(Kind::Composed { text: text.to_string(), formula, parts })
        }
        (None, _) => Ok(Kind::Query(Box::new(spec_of(doc)?), conversion_of(doc)?)),
    }
}

/// What certification fixes, whatever the metric is.
pub(crate) fn fingerprint_of(kind: &Kind) -> String {
    match kind {
        // A metric that does not convert keeps the fingerprint it was certified with.
        Kind::Query(spec, None) => fingerprint(spec),
        Kind::Query(spec, Some(c)) => hex::encode(Sha256::digest(
            format!("{CONVERT_RULE}\n{}\n{}\n{}\n{}\n{}", fingerprint(spec), c.to, c.currency_field, c.date_field, c.places).as_bytes(),
        )),
        Kind::Composed { text, .. } => {
            hex::encode(Sha256::digest(format!("ferrum.metrics.composed/1\n{}", text.split_whitespace().collect::<String>()).as_bytes()))
        }
    }
}

/// What a converted metric's fingerprint names besides its query: how it converts. Certifying fixes the
/// rule with the definition, so another rule (`/2`) puts a metric back in draft rather than changing
/// what a certified number means.
const CONVERT_RULE: &str =
    "ferrum.metrics.convert/1: each row at the latest rate valid on its day, direct or else inverse, exact products, each total rounded half to even";

/// Most rates one pair of currencies may have for a read to take them all (the kernel's most rows).
const MOST_RATES: u32 = 100_000;

/// How a metric summing amounts in several currencies reads them in one: each row at the rate valid on
/// its own day (the latest from that day or before, direct or else inverse, as the platform converts
/// documents: `ferrum_domain::util::exchange_rate`), products kept exact, each total rounded half to
/// even to `places`. A row in a currency without a rate by its day leaves the metric without a value,
/// said so, never a total without that row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Conversion {
    pub to: Uuid,
    pub currency_field: String,
    pub date_field: String,
    pub places: u32,
}

pub(crate) fn conversion_of(doc: &Doc) -> Result<Option<Conversion>> {
    let set = |f: &str| s(doc, f).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let (currency_field, date_field) = (set("currency_field"), set("rate_date_field"));
    let Some(to) = set("convert_to") else {
        return match currency_field.or(date_field) {
            Some(_) => Err(Error::field("convert_to", "say which currency to convert to, or clear the currency field and the rate date")),
            None => Ok(None),
        };
    };
    let to = to.parse().map_err(|_| Error::field("convert_to", "is not a currency"))?;
    let currency_field = currency_field.ok_or_else(|| Error::field("currency_field", "say which field holds each row's currency: currency"))?;
    let date_field = date_field.ok_or_else(|| Error::field("rate_date_field", "say which date picks each row's rate: posting_date"))?;
    let places = match doc.get("round_to") {
        None | Some(Value::Null) => 2,
        Some(v) => v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()).filter(|p| *p <= 6).ok_or_else(|| Error::field("round_to", "is 0 to 6 places"))?,
    };
    Ok(Some(Conversion { to, currency_field, date_field, places: u32::try_from(places).unwrap_or(2) }))
}

/// A converted total, rounded half to even (a tie goes to the even digit, so ties do not all lean one way).
fn rounded(v: Decimal, places: u32) -> Decimal {
    v.round_dp_with_strategy(places, RoundingStrategy::MidpointNearestEven)
}

/// Amounts added exactly, or no value rather than a wrong one when they outgrow what a decimal holds.
fn total(me: &str, values: impl IntoIterator<Item = Decimal>) -> Result<Decimal> {
    values
        .into_iter()
        .try_fold(Decimal::ZERO, |t, v| t.checked_add(v))
        .ok_or_else(|| Error::invalid(format!("{me} has no value: its total is too large to hold exactly")))
}

/// A cell's number, exact; nothing counted is zero.
fn number(v: &Value) -> Decimal {
    match v {
        Value::String(s) => s.parse().unwrap_or_default(),
        Value::Number(n) => n.to_string().parse().unwrap_or_default(),
        _ => Decimal::ZERO,
    }
}

/// The record a field path of `entity` ends at, through links and child tables: `customer.currency`.
fn path_target(ctx: &Ctx, entity: &str, path: &str) -> Option<String> {
    let mut at = entity.to_string();
    for part in path.split('.') {
        at = reached(ctx, &at, part)?;
    }
    Some(at)
}

/// A converted metric's query summed by currency and by the day picking the rate, and by `also`'s day
/// (a series going by another date) when given: as many groups as one read holds.
fn grouped(spec: &Spec, c: &Conversion, also: Option<&str>) -> Spec {
    let mut g = spec.clone();
    let by_day = |f: &str| GroupBy { field: f.to_string(), bucket: Some(Bucket::Day), label: None };
    g.group_by = vec![GroupBy { field: c.currency_field.clone(), bucket: None, label: None }, by_day(&c.date_field)];
    g.group_by.extend(also.map(by_day));
    g.limit = Some(MAX_LIMIT);
    g.sort.clear();
    g
}

/// A read holding every row it was asked for: one cut short at Studio's most rows would give a total
/// without some of them, so it is refused instead.
fn whole(out: Value, me: &str) -> Result<Value> {
    if out["meta"]["truncated"] == json!(true) {
        return Err(Error::invalid(format!(
            "{me} has no value: it converts more than {MAX_LIMIT} currency-days in one read; narrow its definition with a date range"
        )));
    }
    Ok(out)
}

/// One group of a converted metric's rows: its currency, the day picking its rate, the day its period
/// goes by (the same unless a series goes by another date), and its sum.
struct Group {
    currency: Uuid,
    rate_day: NaiveDate,
    day: NaiveDate,
    sum: Decimal,
}

/// A converted metric's rows by currency and day, for `ctx` with its own rights, only from `range` of
/// the day a series goes by when given.
async fn groups(k: &Kernel, ctx: &Ctx, me: &str, spec: &Spec, c: &Conversion, range: Option<(&str, NaiveDate, NaiveDate)>) -> Result<Vec<Group>> {
    let also = range.map(|(f, ..)| f).filter(|f| *f != c.date_field);
    let mut g = grouped(spec, c, also);
    if let Some((f, from, to)) = range {
        let after = to.succ_opt().unwrap_or(to);
        g.and_filters(&json!([[f, ">=", from.to_string()], [f, "<", after.to_string()]]))?;
    }
    let out = whole(ferrum_studio::run::run(k, ctx, &g).await?, me)?;
    let measure = out["columns"].as_array().and_then(|c| c.last()).and_then(|c| c["name"].as_str()).unwrap_or_default().to_string();
    let rate_col = format!("day({})", c.date_field);
    let day_col = also.map_or_else(|| rate_col.clone(), |f| format!("day({f})"));
    let missing = |what: &str| Error::invalid(format!("{me} has no value: some of its rows have no {what}"));
    let day = |v: &Value| v.as_str().and_then(|t| t.get(..10)?.parse::<NaiveDate>().ok());
    out["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            Ok(Group {
                currency: r[&c.currency_field].as_str().and_then(|v| v.parse().ok()).ok_or_else(|| missing(&c.currency_field))?,
                rate_day: day(&r[&rate_col]).ok_or_else(|| missing(&c.date_field))?,
                day: day(&r[&day_col]).ok_or_else(|| missing(also.unwrap_or(&c.date_field)))?,
                sum: number(&r[&measure]),
            })
        })
        .collect()
}

/// Every rate from `from` to `to`, by the day it is valid from, oldest first: all of them, or refused.
async fn rates(k: &Kernel, system: &Ctx, tx: &mut Tx, from: Uuid, to: Uuid) -> Result<Vec<(NaiveDate, Decimal)>> {
    let pair = [Filter::eq("from_currency", from.to_string()), Filter::eq("to_currency", to.to_string())];
    let found = k.find_tx(system, tx, "ExchangeRate", &pair, Some("valid_from asc"), MOST_RATES).await?;
    if found.len() >= MOST_RATES as usize {
        return Err(Error::invalid(format!("more than {MOST_RATES} exchange rates between two currencies: too many to read at once")));
    }
    Ok(found.iter().filter_map(|r| Some((s(r, "valid_from")?.get(..10)?.parse().ok()?, ferrum_domain::util::d(r, "rate")))).collect())
}

/// The rate on `day`: the latest direct one valid from that day or before, or else the inverse of the
/// latest inverse one, exactly as `ferrum_domain::util::exchange_rate` takes it for a document.
fn rate_on(direct: &[(NaiveDate, Decimal)], inverse: &[(NaiveDate, Decimal)], day: NaiveDate) -> Option<Decimal> {
    let latest = |list: &[(NaiveDate, Decimal)]| list.partition_point(|(from, _)| *from <= day).checked_sub(1).map(|i| list[i].1);
    latest(direct).or_else(|| latest(inverse).filter(|r| !r.is_zero()).map(|r| (Decimal::ONE / r).round_dp(9)))
}

/// A currency's code, for saying which rate is missing.
async fn code(k: &Kernel, system: &Ctx, tx: &mut Tx, currency: Uuid) -> String {
    match k.get_tx(system, tx, "Currency", currency, false).await {
        Ok(c) => s(&c, "code").unwrap_or("a currency").to_string(),
        Err(_) => currency.to_string(),
    }
}

/// Each group's sum in the metric's currency, with the day its period goes by: exact, or no value when
/// a group has no rate by its day. The rates are what the metric means, so they are read whoever reads it.
async fn convert(k: &Kernel, ctx: &Ctx, me: &str, c: &Conversion, groups: Vec<Group>) -> Result<Vec<(NaiveDate, Decimal)>> {
    let system = sys(ctx);
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let mut known: HashMap<Uuid, (Vec<(NaiveDate, Decimal)>, Vec<(NaiveDate, Decimal)>)> = HashMap::new();
    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        if g.currency == c.to {
            out.push((g.day, g.sum));
            continue;
        }
        let (direct, inverse) = match known.entry(g.currency) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert((rates(k, &system, &mut tx, g.currency, c.to).await?, rates(k, &system, &mut tx, c.to, g.currency).await?)),
        };
        let Some(rate) = rate_on(direct, inverse, g.rate_day) else {
            let (from, to) = (code(k, &system, &mut tx, g.currency).await, code(k, &system, &mut tx, c.to).await);
            return Err(Error::invalid(format!(
                "{me} has no value: no exchange rate from {from} to {to} on or before {}; add one under Exchange Rate",
                g.rate_day
            )));
        };
        let amount = g.sum.checked_mul(rate).ok_or_else(|| Error::invalid(format!("{me} has no value: an amount is too large to convert exactly")))?;
        out.push((g.day, amount));
    }
    tx.commit().await?;
    Ok(out)
}

/// A converted metric checked when it is saved: it sums an amount, by a field linking each row to its
/// currency and a date picking the rate, into a currency that exists, and grouped so its query still
/// makes sense to its author's rights (a currency on child rows would count the parent once per row).
async fn check_conversion(hc: &mut HookCtx<'_>, spec: &Spec, c: &Conversion) -> Result<()> {
    if spec.measures.first().map(|m| m.func) != Some(Agg::Sum) {
        return Err(Error::field("convert_to", "a converted metric sums an amount: counts, averages, least and most are not converted"));
    }
    if path_target(hc.ctx, &spec.entity, &c.currency_field).as_deref() != Some("Currency") {
        return Err(Error::field("currency_field", format!("{} does not link to a currency", c.currency_field)));
    }
    ferrum_studio::query::compile(hc.ctx, &grouped(spec, c, None)).map_err(|e| Error::field("definition", format!("does not run by currency and day: {e}")))?;
    if hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "Currency", c.to, false).await.is_err() {
        return Err(Error::field("convert_to", "is not a currency here"));
    }
    Ok(())
}

/// What a metric meant when it was read, kept with the value: its query or formula, and how it converts.
pub(crate) fn meaning(doc: &Doc) -> Value {
    let keys = ["definition", "formula", "convert_to", "currency_field", "rate_date_field", "round_to"];
    Value::Object(keys.iter().filter_map(|k| doc.get(*k).filter(|v| !v.is_null() && *v != &json!("")).map(|v| ((*k).to_string(), v.clone()))).collect())
}

/// The metric named `name` in `ctx`'s workspace, whoever may read it.
pub(crate) async fn named(k: &Kernel, ctx: &Ctx, name: &str) -> Result<Option<Doc>> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let found = k.find_tx(&ferrum_domain::util::sys(ctx), &mut tx, "Metric", &[ferrum_kernel::Filter::eq("metric_name", name)], None, 1).await?;
    tx.commit().await?;
    Ok(found.into_iter().next())
}

/// Every metric that names `name` in its formula, and every one that names those, by name: what a
/// change to it reaches (impact).
pub(crate) async fn used_by(k: &Kernel, ctx: &Ctx, name: &str) -> Result<Vec<String>> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let all: Vec<(String, Option<String>)> =
        sqlx::query_as("select metric_name, formula from e_metric where tenant_id = $1 and coalesce(formula, '') <> '' limit 5000")
            .bind(ctx.tenant)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let uses: BTreeMap<String, Vec<String>> = all.into_iter().filter_map(|(n, f)| Some((n, parts_of(f.as_deref()?).ok()?))).collect();
    let (mut out, mut todo) = (BTreeSet::new(), vec![name.to_string()]);
    while let Some(part) = todo.pop() {
        for (m, parts) in &uses {
            if parts.contains(&part) && out.insert(m.clone()) {
                todo.push(m.clone());
            }
        }
    }
    out.remove(name);
    Ok(out.into_iter().collect())
}

/// A formula's parts must be metrics, never itself, even by way of others, none deprecated, and those
/// that convert money all into one currency.
async fn check_parts(k: &Kernel, ctx: &Ctx, me: &str, parts: &[String]) -> Result<()> {
    let mut todo: Vec<(String, Vec<String>)> = parts.iter().map(|p| (p.clone(), vec![me.to_string(), p.clone()])).collect();
    let mut currencies: BTreeMap<Uuid, String> = BTreeMap::new();
    while let Some((part, path)) = todo.pop() {
        if part.eq_ignore_ascii_case(me) {
            return Err(Error::field("formula", format!("a metric is not made of itself, even by way of others: {}", path.join(" → "))));
        }
        if path.len() > MAX_DEPTH + 1 {
            return Err(Error::field("formula", format!("metrics are composed {MAX_DEPTH} deep at most")));
        }
        let Some(doc) = named(k, ctx, &part).await? else {
            return Err(Error::field("formula", format!("there is no metric called {part}")));
        };
        if s(&doc, "status") == Some("Deprecated") {
            return Err(Error::field("formula", format!("{part} is deprecated")));
        }
        if let Ok(Some(c)) = conversion_of(&doc) {
            currencies.entry(c.to).or_insert_with(|| part.clone());
            if currencies.len() > 1 {
                let names: Vec<&str> = currencies.values().map(String::as_str).collect();
                return Err(Error::field("formula", format!("{} convert to different currencies: a formula keeps to one", names.join(" and "))));
            }
        }
        if let Some(text) = s(&doc, "formula").filter(|f| !f.trim().is_empty()) {
            for next in parts_of(text)? {
                let mut p = path.clone();
                p.push(next.clone());
                todo.push((next, p));
            }
        }
    }
    Ok(())
}

/// The definition's fingerprint: what certification fixes.
pub(crate) fn fingerprint(spec: &Spec) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(spec).unwrap_or_default()))
}

/// What a query reads: its record's fields, then each linked record's (or child table's) fields
/// under the record it reaches by which field: "SalesInvoice: grand_total; Customer (by customer):
/// customer_group". `target` names the record a field of the query's record reaches.
pub(crate) fn lineage(spec: &Spec, target: impl Fn(&str) -> Option<String>) -> String {
    let mut paths = BTreeSet::new();
    paths.extend(spec.measures.iter().filter_map(|m| m.field.clone()));
    paths.extend(spec.fields.iter().map(|f| f.field.clone()));
    paths.extend(spec.group_by.iter().map(|g| g.field.clone()));
    paths.extend(spec.pivot.iter().map(|p| p.field.clone()));
    paths.extend(spec.time_series.iter().map(|t| t.field.clone()));
    paths.extend(spec.date_range.iter().map(|d| d.field.clone()));
    if let Ok(filters) = ferrum_kernel::query::parse_filters(&spec.filters) {
        paths.extend(filters.into_iter().map(|f| f.field));
    }
    let (mut own, mut reached) = (BTreeSet::new(), BTreeMap::<String, BTreeSet<String>>::new());
    for p in paths {
        match p.split_once('.') {
            Some((via, rest)) => {
                let record = target(via).map_or_else(|| format!("by {via}"), |t| format!("{t} (by {via})"));
                reached.entry(record).or_default().insert(rest.to_string());
            }
            None => {
                own.insert(p);
            }
        }
    }
    let list = |s: BTreeSet<String>| s.into_iter().collect::<Vec<_>>().join(", ");
    let mut out = if own.is_empty() { spec.entity.clone() } else { format!("{}: {}", spec.entity, list(own)) };
    for (record, fields) in reached {
        out = format!("{out}; {record}: {}", list(fields));
    }
    out
}

/// The record a field of `entity` reaches, by link or as a child table, in `ctx`'s schema.
pub(crate) fn reached(ctx: &Ctx, entity: &str, field: &str) -> Option<String> {
    let f = ctx.schema.entity(entity)?.field(field)?;
    f.to.clone().or_else(|| f.of.clone())
}

/// Whoever certifies and deprecates: a Data Steward, or an administrator.
fn steward(ctx: &Ctx) -> bool {
    ctx.is_admin || ctx.roles.iter().any(|r| r == "Data Steward")
}

/// A certified definition that changed is a draft again, to be certified anew.
pub(crate) fn redraft(doc: &mut Doc, hash: &str, also: &[&str]) {
    if s(doc, "status") == Some("Certified") && s(doc, "certified_hash") != Some(hash) {
        doc.insert("status".into(), json!("Draft"));
        for k in ["certified_by", "certified_at", "certified_hash"].iter().chain(also) {
            doc.insert((*k).into(), Value::Null);
        }
    }
}

/// `certify` on a metric or a dataset: by a steward who does not own it, fixing the definition's
/// fingerprint `hash`, with `more` kept beside it.
pub(crate) async fn certify(hc: &mut HookCtx<'_>, entity: &str, doc: &Doc, hash: String, more: Value) -> Result<Value> {
    let what = entity.to_lowercase();
    if !steward(hc.ctx) {
        return Err(Error::Forbidden(format!("a Data Steward certifies a {what}")));
    }
    if s(doc, "owner").is_some_and(|o| o.eq_ignore_ascii_case(&hc.ctx.email)) {
        return Err(Error::Forbidden(format!("a {what} is certified by someone other than its owner")));
    }
    let record = id(doc, "id").ok_or_else(|| Error::internal("a record without an id"))?;
    let mut values = json!({"status": "Certified", "certified_by": hc.ctx.email, "certified_at": Utc::now().to_rfc3339(), "certified_hash": hash});
    if let (Value::Object(v), Value::Object(m)) = (&mut values, more) {
        v.extend(m);
    }
    hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, entity, record, obj(values)).await?;
    Ok(json!({"status": "Certified"}))
}

/// `deprecate` on a metric or a dataset: by a steward, naming what replaces it, never itself.
pub(crate) async fn deprecate(hc: &mut HookCtx<'_>, entity: &str, doc: &Doc, params: &Value) -> Result<Value> {
    if !steward(hc.ctx) {
        return Err(Error::Forbidden(format!("a Data Steward deprecates a {}", entity.to_lowercase())));
    }
    let record = id(doc, "id").ok_or_else(|| Error::internal("a record without an id"))?;
    let replaced: Option<Uuid> = params.get("replaced_by").and_then(Value::as_str).and_then(|v| v.parse().ok());
    if replaced == Some(record) {
        return Err(Error::field("replaced_by", format!("a {} does not replace itself", entity.to_lowercase())));
    }
    let values = json!({"status": "Deprecated", "replaced_by": replaced.map(|r| r.to_string())});
    hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, entity, record, obj(values)).await?;
    Ok(json!({"status": "Deprecated"}))
}

/// The metric's value for `ctx`, with its rights to the records the query counts. A composed metric is
/// worked out from each part's value for the same person, so two people may read two numbers from one
/// definition; a part not certified, or a division by zero, leaves it without a value, said so.
pub(crate) async fn value(k: &Kernel, ctx: &Ctx, doc: &Doc) -> Result<Decimal> {
    value_at(k, ctx, doc, 0).await
}

type Valued<'a> = Pin<Box<dyn Future<Output = Result<Decimal>> + Send + 'a>>;

fn value_at<'a>(k: &'a Kernel, ctx: &'a Ctx, doc: &'a Doc, depth: usize) -> Valued<'a> {
    Box::pin(async move {
        let me = s(doc, "metric_name").unwrap_or("this metric").to_string();
        match kind_of(doc)? {
            Kind::Query(spec, None) => query_value(k, ctx, &spec).await,
            Kind::Query(spec, Some(c)) => {
                let converted = convert(k, ctx, &me, &c, groups(k, ctx, &me, &spec, &c, None).await?).await?;
                Ok(rounded(total(&me, converted.into_iter().map(|(_, v)| v))?, c.places))
            }
            Kind::Composed { formula, parts, .. } => {
                if depth >= MAX_DEPTH {
                    return Err(Error::invalid(format!("{me} is composed more than {MAX_DEPTH} deep")));
                }
                let mut row = serde_json::Map::new();
                for part in &parts {
                    let found = named(k, ctx, part).await?.ok_or_else(|| Error::invalid(format!("{me} has no value: there is no metric called {part}")))?;
                    if s(&found, "status") != Some("Certified") {
                        return Err(Error::invalid(format!("{me} has no value: {part} is not certified")));
                    }
                    let v = value_at(k, ctx, &found, depth + 1).await?;
                    row.insert(part.clone(), json!(v.to_string()));
                }
                formula.eval(&row).ok_or_else(|| Error::invalid(format!("{me} has no value now: a part it divides by is zero, or empty")))
            }
        }
    })
}

/// What a query of one measure counts for `ctx`.
async fn query_value(k: &Kernel, ctx: &Ctx, spec: &Spec) -> Result<Decimal> {
    let out = ferrum_studio::run::run(k, ctx, spec).await?;
    let key = out["columns"].as_array().and_then(|c| c.last()).and_then(|c| c["name"].as_str()).unwrap_or_default().to_string();
    let cell = out["rows"].as_array().and_then(|r| r.first()).map(|r| r[&key].clone()).unwrap_or(Value::Null);
    Ok(number(&cell))
}

/// One period of a metric's series. `partial`: the period is not over, so its value is so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Point {
    pub start: String,
    pub value: Option<String>,
    pub partial: bool,
}

/// `day` moved by `n` periods of `bucket`, forward or (negative) back. From a period's first day, the
/// first day of another.
fn shift(bucket: Bucket, day: NaiveDate, n: i32) -> Option<NaiveDate> {
    use chrono::{Days, Months};
    let (days, months) = match bucket {
        Bucket::Day => (1, 0),
        Bucket::Week => (7, 0),
        Bucket::Month => (0, 1),
        Bucket::Quarter | Bucket::FiscalQuarter => (0, 3),
        Bucket::Year | Bucket::FiscalYear => (0, 12),
    };
    let (days, months) = (Days::new(u64::from(days * n.unsigned_abs())), Months::new(months * n.unsigned_abs()));
    if n >= 0 { day.checked_add_days(days)?.checked_add_months(months) } else { day.checked_sub_days(days)?.checked_sub_months(months) }
}

/// The first day of the period `back` periods before the one holding `today`, fiscal periods starting in
/// the workspace's fiscal start month `fy` (as Studio buckets them).
pub(crate) fn period_start(bucket: Bucket, today: NaiveDate, back: u32, fy: u32) -> Option<NaiveDate> {
    shift(bucket, bucket.start(today, fy), -i32::try_from(back).ok()?)
}

type Series<'a> = Pin<Box<dyn Future<Output = Result<BTreeMap<String, Option<Decimal>>>> + Send + 'a>>;

/// A metric's value for each of the last `periods` periods, the current one marked partial: by its
/// `series_by` date field for a query, period by period from its parts for a composed metric.
pub(crate) async fn series(k: &Kernel, ctx: &Ctx, doc: &Doc, bucket: Bucket, periods: u32) -> Result<Vec<Point>> {
    if !(1..=120).contains(&periods) {
        return Err(Error::field("periods", "is 1 to 120"));
    }
    let (today, fy) = (Utc::now().date_naive(), ctx.fy_start_month);
    let from = period_start(bucket, today, periods - 1, fy).ok_or_else(|| Error::field("periods", "reach further back than dates go"))?;
    let current = period_start(bucket, today, 0, fy).unwrap_or(today).to_string();
    let values = series_at(k, ctx, doc, bucket, from, today, 0).await?;
    Ok(values.into_iter().map(|(start, v)| Point { partial: start >= current, start, value: v.map(|d| d.normalize().to_string()) }).collect())
}

fn series_at<'a>(k: &'a Kernel, ctx: &'a Ctx, doc: &'a Doc, bucket: Bucket, from: NaiveDate, to: NaiveDate, depth: usize) -> Series<'a> {
    Box::pin(async move {
        let me = s(doc, "metric_name").unwrap_or("this metric").to_string();
        let series_by = || {
            s(doc, "series_by")
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .ok_or_else(|| Error::field("series_by", format!("{me}: say which date field a series of it goes by")))
        };
        match kind_of(doc)? {
            // Converted day by day, then each period's total rounded.
            Kind::Query(spec, Some(c)) => {
                let field = series_by()?;
                let converted = convert(k, ctx, &me, &c, groups(k, ctx, &me, &spec, &c, Some((field, from, to))).await?).await?;
                let mut by_period: BTreeMap<String, Vec<Decimal>> = BTreeMap::new();
                let mut start = Some(from);
                while let Some(d) = start.filter(|d| *d <= to) {
                    by_period.insert(d.to_string(), vec![]);
                    start = shift(bucket, d, 1);
                }
                for (day, v) in converted {
                    by_period.entry(bucket.start(day, ctx.fy_start_month).to_string()).or_default().push(v);
                }
                by_period.into_iter().map(|(start, values)| Ok((start, Some(rounded(total(&me, values)?, c.places))))).collect()
            }
            Kind::Query(mut spec, None) => {
                let field = series_by()?;
                spec.time_series =
                    Some(TimeSeries { field: field.trim().to_string(), bucket, from: Some(from.to_string()), to: Some(to.to_string()), fill_zero: true });
                let out = ferrum_studio::run::run(k, ctx, &spec).await?;
                let columns = out["columns"].as_array().cloned().unwrap_or_default();
                let key = columns.last().and_then(|c| c["name"].as_str()).unwrap_or_default().to_string();
                // The period's start is under the column that goes by a bucket.
                let period = columns.iter().find(|c| c.get("bucket").is_some()).and_then(|c| c["name"].as_str()).unwrap_or("bucket").to_string();
                Ok(out["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        let start = r[&period].as_str()?.chars().take(10).collect::<String>();
                        let v = match &r[&key] {
                            Value::String(s) => s.parse().ok(),
                            Value::Number(n) => n.to_string().parse().ok(),
                            _ => Some(Decimal::ZERO),
                        };
                        Some((start, v))
                    })
                    .collect())
            }
            Kind::Composed { formula, parts, .. } => {
                if depth >= MAX_DEPTH {
                    return Err(Error::invalid(format!("{me} is composed more than {MAX_DEPTH} deep")));
                }
                let mut each: Vec<(String, BTreeMap<String, Option<Decimal>>)> = Vec::new();
                for part in &parts {
                    let found = named(k, ctx, part).await?.ok_or_else(|| Error::invalid(format!("{me} has no series: there is no metric called {part}")))?;
                    if s(&found, "status") != Some("Certified") {
                        return Err(Error::invalid(format!("{me} has no series: {part} is not certified")));
                    }
                    each.push((part.clone(), series_at(k, ctx, &found, bucket, from, to, depth + 1).await?));
                }
                let starts: BTreeSet<String> = each.iter().flat_map(|(_, s)| s.keys().cloned()).collect();
                Ok(starts
                    .into_iter()
                    .map(|start| {
                        let mut row = serde_json::Map::new();
                        for (part, values) in &each {
                            if let Some(Some(v)) = values.get(&start) {
                                row.insert(part.clone(), json!(v.to_string()));
                            }
                        }
                        let v = formula.eval(&row);
                        (start, v)
                    })
                    .collect())
            }
        }
    })
}

pub struct MetricHook;

#[async_trait]
impl DocHook for MetricHook {
    fn name(&self) -> &'static str {
        "metrics.metric"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["owner", "uses", "status", "certified_by", "certified_at", "certified_hash"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "owner").is_none() && !hc.ctx.email.is_empty() {
            doc.insert("owner".into(), json!(hc.ctx.email.to_lowercase()));
        }
        let Ok(kind) = kind_of(doc) else { return Ok(()) };
        let uses = match &kind {
            Kind::Query(spec, None) => lineage(spec, |f| reached(hc.ctx, &spec.entity, f)),
            Kind::Query(spec, Some(c)) => {
                let read = lineage(spec, |f| reached(hc.ctx, &spec.entity, f));
                format!("{read}; converted by {} at the rate on {}", c.currency_field, c.date_field)
            }
            Kind::Composed { parts, .. } => format!("Composed of: {}", parts.join(", ")),
        };
        doc.insert("uses".into(), json!(uses));
        redraft(doc, &fingerprint_of(&kind), &[]);
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        match kind_of(doc)? {
            // The query must make sense to the records and to its author's rights.
            Kind::Query(spec, conversion) => {
                ferrum_studio::query::compile(hc.ctx, &spec).map_err(|e| Error::field("definition", format!("does not run: {e}")))?;
                if let Some(c) = conversion {
                    check_conversion(hc, &spec, &c).await?;
                }
            }
            Kind::Composed { parts, .. } => check_parts(hc.kernel, hc.ctx, s(doc, "metric_name").unwrap_or_default(), &parts).await?,
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        let metric = id(doc, "id").ok_or_else(|| Error::internal("a metric without an id"))?;
        let system = elevated(hc.ctx);
        match name {
            "certify" => {
                let kind = kind_of(doc)?;
                // A composed metric is certified after its parts: what it means is what they mean.
                if let Kind::Composed { parts, .. } = &kind {
                    for part in parts {
                        let found = named(hc.kernel, hc.ctx, part).await?;
                        if found.as_ref().and_then(|d| s(d, "status")) != Some("Certified") {
                            return Err(Error::invalid(format!("{part} is certified first: what this means is what its parts mean")));
                        }
                    }
                }
                certify(hc, "Metric", doc, fingerprint_of(&kind), json!({})).await.map(Some)
            }
            "deprecate" => deprecate(hc, "Metric", doc, params).await.map(Some),
            "snapshot" => {
                let v = value(hc.kernel, hc.ctx, doc).await?;
                hc.kernel
                    .insert_tx(
                        &system,
                        hc.tx,
                        "MetricSnapshot",
                        obj(json!({"metric": metric.to_string(), "value": ferrum_domain::util::money(v), "read_by": hc.ctx.email, "read_at": Utc::now().to_rfc3339(),
                                   "certified_hash": s(doc, "certified_hash"), "definition": meaning(doc)})),
                    )
                    .await?;
                Ok(Some(json!({"value": ferrum_domain::util::money(v)})))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_metric_is_one_measure_and_its_lineage() {
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let ok = doc(json!({"definition": {"entity": "Customer", "filters": [["customer_type", "eq", "Company"]], "measures": [{"fn": "count"}]}}));
        let spec = spec_of(&ok).unwrap();
        assert_eq!(lineage(&spec, |_| None), "Customer: customer_type");
        let summed = doc(json!({"definition": {"entity": "SalesInvoice", "filters": {"status": "Paid"}, "measures": [{"fn": "sum", "field": "grand_total"}]}}));
        assert_eq!(lineage(&spec_of(&summed).unwrap(), |_| None), "SalesInvoice: grand_total, status");
        let pair = doc(json!({"definition": {"entity": "Customer", "filters": [["customer_group", "Retail"]], "measures": [{"fn": "count"}]}}));
        assert_eq!(lineage(&spec_of(&pair).unwrap(), |_| None), "Customer: customer_group");
        // Linked records' fields, under the record each link reaches.
        let joined = parse_spec(&doc(json!({"definition": {"entity": "SalesInvoice", "filters": [["customer.customer_group", "eq", "Retail"]],
            "group_by": [{"field": "customer.territory"}, {"field": "posting_date"}], "measures": [{"fn": "sum", "field": "grand_total"}]}})))
        .unwrap();
        let target = |f: &str| (f == "customer").then(|| "Customer".to_string());
        assert_eq!(lineage(&joined, target), "SalesInvoice: grand_total, posting_date; Customer (by customer): customer_group, territory");
        let two = doc(json!({"definition": {"entity": "Customer", "measures": [{"fn": "count"}, {"fn": "count"}]}}));
        assert!(spec_of(&two).is_err());
        let grouped = doc(json!({"definition": {"entity": "Customer", "group_by": [{"field": "customer_type"}], "measures": [{"fn": "count"}]}}));
        assert!(spec_of(&grouped).is_err());
        let pivoted = doc(json!({"definition": {"entity": "Customer", "pivot": {"field": "customer_type"}, "measures": [{"fn": "count"}]}}));
        assert!(spec_of(&pivoted).is_err());
        let series = doc(json!({"definition": {"entity": "Customer", "time_series": {"field": "creation", "bucket": "month"}, "measures": [{"fn": "count"}]}}));
        assert!(spec_of(&series).is_err());
        assert_ne!(fingerprint(&spec), fingerprint(&spec_of(&doc(json!({"definition": {"entity": "Customer", "measures": [{"fn": "count"}]}}))).unwrap()));
    }

    #[test]
    fn conversion_is_what_certification_fixes_and_nothing_else_changes() {
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let plain = doc(json!({"definition": {"entity": "Sale", "measures": [{"fn": "sum", "field": "amount"}]}, "round_to": 2}));
        let spec = spec_of(&plain).unwrap();
        // A metric that does not convert keeps the fingerprint it was certified with before conversion existed.
        assert_eq!(fingerprint_of(&kind_of(&plain).unwrap()), fingerprint(&spec));
        let inr = "0195f5a4-0000-7000-8000-000000000001";
        let mut converted = plain.clone();
        converted.extend(doc(json!({"convert_to": inr, "currency_field": "currency", "rate_date_field": "posted_on"})));
        let printed = fingerprint_of(&kind_of(&converted).unwrap());
        assert_ne!(printed, fingerprint(&spec), "how it converts is part of what it means");
        let mut finer = converted.clone();
        finer.insert("round_to".into(), json!(4));
        assert_ne!(fingerprint_of(&kind_of(&finer).unwrap()), printed, "and so is its rounding");
        let mut other_day = converted.clone();
        other_day.insert("rate_date_field".into(), json!("due_on"));
        assert_ne!(fingerprint_of(&kind_of(&other_day).unwrap()), printed);
        // Half set, or rounded beyond six places, is refused.
        let half = |k: &str| {
            let mut d = plain.clone();
            d.insert(k.into(), json!("x"));
            conversion_of(&d).unwrap_err().to_string()
        };
        assert!(half("currency_field").contains("say which currency"));
        assert!(half("rate_date_field").contains("say which currency"));
        let mut no_day = converted.clone();
        no_day.remove("rate_date_field");
        assert!(conversion_of(&no_day).is_err());
        let mut seven = converted.clone();
        seven.insert("round_to".into(), json!(7));
        assert!(conversion_of(&seven).is_err());
        assert_eq!(conversion_of(&converted).unwrap().map(|c| c.places), Some(2));
    }

    #[test]
    fn periods_fiscal_and_calendar() {
        let d = |s: &str| s.parse::<NaiveDate>().unwrap();
        let today = d("2026-11-08");
        let at = |b: Bucket, back: u32| period_start(b, today, back, 4).unwrap().to_string();
        assert_eq!((at(Bucket::FiscalYear, 0), at(Bucket::FiscalYear, 1)), ("2026-04-01".into(), "2025-04-01".into()));
        assert_eq!(
            (at(Bucket::FiscalQuarter, 0), at(Bucket::FiscalQuarter, 1), at(Bucket::FiscalQuarter, 3)),
            ("2026-10-01".into(), "2026-07-01".into(), "2026-01-01".into())
        );
        assert_eq!(period_start(Bucket::FiscalYear, d("2026-03-31"), 0, 4).unwrap(), d("2025-04-01"), "the last day of a fiscal year is in it");
        assert_eq!((at(Bucket::Day, 3), at(Bucket::Week, 1), at(Bucket::Month, 2)), ("2026-11-05".into(), "2026-10-26".into(), "2026-09-01".into()));
        assert_eq!((at(Bucket::Quarter, 1), at(Bucket::Year, 1)), ("2026-07-01".into(), "2025-01-01".into()));
        assert_eq!(period_start(Bucket::FiscalYear, today, 0, 1).unwrap(), d("2026-01-01"), "a fiscal year from January is the calendar's");
        assert_eq!(shift(Bucket::FiscalQuarter, d("2026-01-01"), 1), Some(d("2026-04-01")));
    }

    #[test]
    fn the_rate_on_a_day_and_rounding_to_even() {
        let d = |s: &str| s.parse::<NaiveDate>().unwrap();
        let n = |s: &str| s.parse::<Decimal>().unwrap();
        let direct = [(d("2026-03-01"), n("83.10")), (d("2026-04-01"), n("84.25"))];
        assert_eq!(rate_on(&direct, &[], d("2026-03-31")), Some(n("83.10")));
        assert_eq!(rate_on(&direct, &[], d("2026-04-01")), Some(n("84.25")), "valid from its own day");
        assert_eq!(rate_on(&direct, &[], d("2026-02-28")), None, "nothing before the first");
        // Only the inverse: its reciprocal to 9 places, as documents take it; a direct one wins.
        let inverse = [(d("2026-01-01"), n("0.0095"))];
        assert_eq!(rate_on(&[], &inverse, d("2026-03-20")), Some(n("105.263157895")));
        assert_eq!(rate_on(&direct, &inverse, d("2026-03-20")), Some(n("83.10")));
        assert_eq!(rate_on(&[], &[(d("2026-01-01"), Decimal::ZERO)], d("2026-03-20")), None);
        assert_eq!((rounded(n("10868.925"), 2), rounded(n("4220.935"), 2), rounded(n("-1.005"), 2)), (n("10868.92"), n("4220.94"), n("-1.00")));
        assert_eq!(rounded(n("13371.55657895"), 0), n("13372"));
        assert!(total("m", [Decimal::MAX, Decimal::ONE]).is_err(), "too large is no value, never a wrong one");
        // A read cut short is refused, never summed.
        assert!(whole(json!({"rows": [], "meta": {"truncated": true}}), "m").is_err());
        assert!(whole(json!({"rows": [], "meta": {"truncated": false}}), "m").is_ok());
    }
}
