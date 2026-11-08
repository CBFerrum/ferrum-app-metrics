//! Wren semantic models (MDL) brought into Metrics as drafts, never as a second authority.
//! `POST /api/app/metrics/import/wren` with `{"mdl": <a Wren manifest>, "map"?: {model: {entity,
//! fields: {column: field}}}, "accept"?: [{"measure": "cube.measure", "as"?: "Metric name"}]}`.
//!
//! Each cube measure becomes a proposed metric over the records its model is mapped to (by `map`, or
//! a record and fields of the same names), compared with the metrics already defined:
//! * `new`: nothing like it here; created when accepted, as a draft that a steward certifies.
//! * `same`: a metric of that name already means exactly this.
//! * `duplicate`: a metric of another name already means exactly this, so it is not defined twice.
//! * `conflict`: a metric of that name means something else. It stays as it is; the import comes in only
//!   under another name the reviewer gives (`as`), and is then a draft beside it.
//! * `unsupported`: no exact meaning here. A calculated column, a relationship, a view or any
//!   expression other than one aggregate of one column is refused, with the reason, never approximated.
//!
//! Wren's own row and column rules are not brought: each reader's rights in Ferrum decide what a metric
//! counts, and the review says so where a model had them. Accepting is all or nothing, in one
//! transaction, with the importer's own rights.

use crate::metric::{fingerprint, fingerprint_of, kind_of};
use crate::obj;
use ferrum_domain::util::{id, s, sys};
use ferrum_kernel::{Access, Action, Ctx, Error, Kernel, Result};
use ferrum_studio::spec::{Agg, Bucket, Measure, Spec, TimeSeries};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Most cubes, measures and models one import may hold.
const MOST_CUBES: usize = 200;
const MOST_MEASURES: usize = 1000;
const MOST_MODELS: usize = 500;
/// Longest expression or name read.
const LONGEST: usize = 2000;
/// Most metrics an import is compared with.
const MOST_DEFINED: u32 = 20_000;

/// One cube measure, as a metric of Metrics or the reason it cannot be one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proposal {
    /// `cube.measure`.
    pub measure: String,
    /// The measure's own name, the metric's unless the reviewer gives another.
    pub name: String,
    pub expression: String,
    pub description: String,
    /// The definition and the date a series goes by, or why there is none.
    pub definition: std::result::Result<(Spec, Option<String>), String>,
    pub notes: Vec<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty() && t.len() <= LONGEST).map(str::to_string)
}

fn list<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(Error::field("mdl", format!("{key} is a list"))),
    }
}

/// A name as Wren and Ferrum both might write it: `sales_order`, `SalesOrder`.
fn plain(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// A bare column of `model`: `amount`, `"amount"`, `sales.amount`; anything else is not one.
fn column_of(expr: &str, model: &str) -> Option<String> {
    let parts: Vec<String> = expr.split('.').map(|p| p.trim().trim_matches('"').to_string()).collect();
    let ident = |p: &str| p.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    match parts.as_slice() {
        [c] if ident(c) => Some(c.clone()),
        [m, c] if ident(m) && ident(c) && m.eq_ignore_ascii_case(model) => Some(c.clone()),
        _ => None,
    }
}

/// One aggregate of one column (or of every row): `sum(amount)`, `count(*)`, `count(distinct customer)`.
/// What else Wren may write has no exact Studio meaning and gives nothing.
pub(crate) fn aggregate(expr: &str, model: &str) -> Option<(Agg, Option<String>)> {
    let e = expr.trim();
    let open = e.find('(')?;
    let inner = e.strip_suffix(')')?.get(open + 1..)?.trim();
    if inner.contains(['(', ')']) {
        return None;
    }
    let func = e[..open].trim().to_ascii_lowercase();
    let distinct = inner.get(..9).is_some_and(|p| p.eq_ignore_ascii_case("distinct "));
    let arg = if distinct { inner.get(9..)?.trim() } else { inner };
    match (func.as_str(), distinct, arg) {
        ("count", false, "*") => Some((Agg::Count, None)),
        ("count", true, c) => Some((Agg::CountDistinct, Some(column_of(c, model)?))),
        (f, false, c) => {
            let agg = match f {
                "count" => Agg::Count,
                "sum" => Agg::Sum,
                "avg" => Agg::Avg,
                "min" => Agg::Min,
                "max" => Agg::Max,
                _ => return None,
            };
            Some((agg, Some(column_of(c, model)?)))
        }
        _ => None,
    }
}

/// How a model's columns become a record's fields: by the reviewer's `map`, else by name.
struct Mapped<'a> {
    model: &'a str,
    entity: String,
    fields: BTreeMap<String, String>,
    /// The model's columns by name (none listed: any name is taken as it is).
    columns: BTreeMap<String, &'a Value>,
}

impl Mapped<'_> {
    /// The field a column is: as mapped, else the record's field of the column's name (or of the source
    /// column a Wren column renames). A calculated column or a relationship is not one.
    fn field(&self, ctx: &Ctx, column: &str) -> std::result::Result<String, String> {
        if let Some(f) = self.fields.get(column) {
            return Ok(f.clone());
        }
        let mut names = vec![column.to_string()];
        if !self.columns.is_empty() {
            let Some(c) = self.columns.get(column) else { return Err(format!("{} has no column {column}", self.model)) };
            if c.get("relationship").and_then(Value::as_str).is_some_and(|r| !r.is_empty()) {
                return Err(format!("{column} is a relationship in Wren: map it to a field, or define the metric by hand"));
            }
            let source = text(c, "expression");
            let renamed = source.as_deref().map(|e| column_of(e, self.model));
            if c.get("isCalculated").and_then(Value::as_bool).unwrap_or(false) || matches!(renamed, Some(None)) {
                return Err(format!("{column} is calculated in Wren ({}): map it to a field, or define the metric by hand", source.unwrap_or_default()));
            }
            names.extend(renamed.flatten());
        }
        let ent = ctx.schema.entity(&self.entity).ok_or_else(|| format!("there is no record {}", self.entity))?;
        names
            .iter()
            .find_map(|n| ent.def.fields.iter().find(|f| plain(&f.name) == plain(n)))
            .map(|f| f.name.clone())
            .ok_or_else(|| format!("{} has no field for column {column}: map it", self.entity))
    }
}

/// Every cube measure of `mdl` as a proposal, mapped by `map` and checked against the records and
/// `ctx`'s rights.
pub(crate) fn proposals(ctx: &Ctx, mdl: &Value, map: &Map<String, Value>) -> Result<Vec<Proposal>> {
    if !mdl.is_object() {
        return Err(Error::field("mdl", "is a Wren manifest: an object with models and cubes"));
    }
    let (models, cubes) = (list(mdl, "models")?, list(mdl, "cubes")?);
    if models.len() > MOST_MODELS || cubes.len() > MOST_CUBES {
        return Err(Error::field("mdl", format!("holds at most {MOST_MODELS} models and {MOST_CUBES} cubes")));
    }
    if cubes.is_empty() {
        return Err(Error::field("mdl", "has no cubes: measures are what become metrics"));
    }
    let by_name: BTreeMap<String, &Value> = models.iter().filter_map(|m| Some((text(m, "name")?, m))).collect();
    let mut out = Vec::new();
    for cube in cubes {
        let cube_name = text(cube, "name").ok_or_else(|| Error::field("mdl", "a cube has no name"))?;
        let base = text(cube, "baseObject").unwrap_or_default();
        let measures = list(cube, "measures")?;
        if out.len() + measures.len() > MOST_MEASURES {
            return Err(Error::field("mdl", format!("holds at most {MOST_MEASURES} measures")));
        }
        // The cube's model, and the record it is mapped to.
        let mut notes = Vec::new();
        let mapped: std::result::Result<Mapped, String> = match by_name.get(&base) {
            None => Err(format!("{base} is not a model: views and other bases are not brought")),
            Some(model) => {
                let given = map.get(&base);
                let entity = given.and_then(|g| text(g, "entity")).or_else(|| ctx.schema.entities.keys().find(|e| plain(e) == plain(&base)).cloned());
                let fields: BTreeMap<String, String> = given
                    .and_then(|g| g.get("fields"))
                    .and_then(Value::as_object)
                    .map(|f| f.iter().filter_map(|(c, v)| Some((c.clone(), v.as_str()?.trim().to_string()))).collect())
                    .unwrap_or_default();
                let columns = list(model, "columns")?.iter().filter_map(|c| Some((text(c, "name")?, c))).collect();
                if !list(model, "rowLevelAccessControls")?.is_empty()
                    || list(model, "columns")?.iter().any(|c| c.get("columnLevelAccessControl").is_some_and(|v| !v.is_null()))
                {
                    notes.push(format!("Wren's row and column rules on {base} are not brought: each reader's own rights here decide what it counts"));
                }
                match entity {
                    Some(entity) if ctx.schema.entity(&entity).is_some() => Ok(Mapped { model: base.as_str(), entity, fields, columns }),
                    Some(entity) => Err(format!("{base} is mapped to {entity}, which is not a record here")),
                    None => Err(format!("{base} is not mapped to a record: map it to one")),
                }
            }
        };
        if !list(cube, "dimensions")?.is_empty() {
            notes.push("its dimensions are not brought: a reader groups a metric's records by any field they may read".into());
        }
        // The date a series goes by: the first time dimension that is a bare column and a date here.
        let series_by = mapped.as_ref().ok().and_then(|m| {
            list(cube, "timeDimensions").ok()?.iter().find_map(|t| {
                let field = m.field(ctx, &column_of(&text(t, "expression")?, m.model)?).ok()?;
                let probe = Spec {
                    entity: m.entity.clone(),
                    measures: vec![Measure { func: Agg::Count, field: None, label: None }],
                    time_series: Some(TimeSeries { field: field.clone(), bucket: Bucket::Month, from: None, to: None, fill_zero: false }),
                    ..Default::default()
                };
                ferrum_studio::query::compile(ctx, &probe).is_ok().then_some(field)
            })
        });
        for m in measures {
            let name = text(m, "name").ok_or_else(|| Error::field("mdl", format!("a measure of {cube_name} has no name")))?;
            let expression = text(m, "expression").unwrap_or_default();
            let definition = mapped.as_ref().map_err(Clone::clone).and_then(|mapped| {
                let (func, column) = aggregate(&expression, mapped.model).ok_or_else(|| {
                    format!("{expression} has no exact meaning here: one aggregate (sum, count, count distinct, avg, min, max) of one column is brought; define anything else by hand")
                })?;
                let field = column.map(|c| mapped.field(ctx, &c)).transpose()?;
                let spec = Spec { entity: mapped.entity.clone(), measures: vec![Measure { func, field, label: None }], ..Default::default() };
                ferrum_studio::query::compile(ctx, &spec).map_err(|e| format!("does not run here: {e}"))?;
                Ok((spec, series_by.clone()))
            });
            let description = match text(m, "description") {
                Some(d) => format!("{d}. Imported from Wren: cube {cube_name}, measure {name} = {expression}"),
                None => format!("Imported from Wren: cube {cube_name}, measure {name} = {expression}"),
            };
            out.push(Proposal { measure: format!("{cube_name}.{name}"), name, expression, description, definition, notes: notes.clone() });
        }
    }
    Ok(out)
}

/// The review of `body`'s model against the metrics defined here, and the accepted measures created as
/// drafts, all or none.
pub(crate) async fn import(k: &Kernel, ctx: &Ctx, body: &[u8]) -> Result<Value> {
    if ctx.perms.access("Metric", Action::Create) == Access::None {
        return Err(Error::Forbidden("a Metric Author imports metrics".into()));
    }
    let body: Value = serde_json::from_slice(body).map_err(|e| Error::invalid(format!("the body is not JSON: {e}")))?;
    let fields = body.as_object().ok_or_else(|| Error::invalid("the body is an object: mdl, map, accept"))?;
    if let Some(other) = fields.keys().find(|k| !["mdl", "map", "accept"].contains(&k.as_str())) {
        return Err(Error::field(other, "is not something an import takes: mdl, map, accept"));
    }
    let map = match fields.get("map") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(Error::field("map", "is an object: {model: {entity, fields: {column: field}}}")),
    };
    let proposals = proposals(ctx, fields.get("mdl").unwrap_or(&Value::Null), &map)?;

    // What is defined here already, by name and by meaning.
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let defined = k.find_tx(&sys(ctx), &mut tx, "Metric", &[], Some("metric_name asc"), MOST_DEFINED).await?;
    tx.commit().await?;
    if defined.len() >= MOST_DEFINED as usize {
        return Err(Error::invalid(format!("more than {MOST_DEFINED} metrics are defined here: too many to compare an import with")));
    }
    let known: Vec<(String, Option<String>, Option<String>)> = defined
        .iter()
        .filter_map(|d| Some((s(d, "metric_name")?.to_string(), kind_of(d).ok().map(|kind| fingerprint_of(&kind)), s(d, "status").map(str::to_string))))
        .collect();
    let mut review = Vec::new();
    for p in &proposals {
        let mut row = json!({"measure": p.measure, "metric_name": p.name, "expression": p.expression, "notes": p.notes});
        match &p.definition {
            Err(why) => {
                row["status"] = json!("unsupported");
                row["reason"] = json!(why);
            }
            Ok((spec, series_by)) => {
                let print = fingerprint(spec);
                row["definition"] = serde_json::to_value(spec).unwrap_or_default();
                row["series_by"] = json!(series_by);
                let named = known.iter().find(|(n, ..)| n.eq_ignore_ascii_case(&p.name));
                let alike = known.iter().find(|(_, f, _)| f.as_deref() == Some(print.as_str()));
                let (status, existing, reason) = match (named, alike) {
                    (Some((n, f, _)), _) if f.as_deref() == Some(print.as_str()) => ("same", Some(n), format!("{n} already means exactly this")),
                    (Some((n, _, st)), _) => (
                        "conflict",
                        Some(n),
                        format!(
                            "{n} is already defined here, differently ({}): it stays as it is; import this under another name or leave it",
                            st.as_deref().unwrap_or("Draft")
                        ),
                    ),
                    (None, Some((n, ..))) => ("duplicate", Some(n), format!("{n} already means exactly this, under another name")),
                    (None, None) => ("new", None, String::new()),
                };
                row["status"] = json!(status);
                row["existing"] = json!(existing);
                if !reason.is_empty() {
                    row["reason"] = json!(reason);
                }
            }
        }
        review.push(row);
    }

    // The accepted: checked all first, then created together.
    let accept = match fields.get("accept") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(Error::field("accept", "is a list of {measure, as}")),
    };
    let mut creating: Vec<(usize, String)> = Vec::new();
    let mut names: BTreeSet<String> = known.iter().map(|(n, ..)| n.to_lowercase()).collect();
    for a in &accept {
        let measure = text(a, "measure").ok_or_else(|| Error::field("accept", "each names a measure: cube.measure"))?;
        if let Some(other) = a.as_object().and_then(|o| o.keys().find(|k| !["measure", "as"].contains(&k.as_str()))) {
            return Err(Error::field("accept", format!("{other} is not something an acceptance takes: measure, as")));
        }
        let i = proposals
            .iter()
            .position(|p| p.measure == measure)
            .ok_or_else(|| Error::field("accept", format!("there is no measure {measure} in this model")))?;
        let given = text(a, "as");
        let status = review[i]["status"].as_str().unwrap_or_default();
        let name = match (status, given) {
            ("new", given) => given.unwrap_or_else(|| proposals[i].name.clone()),
            ("conflict", Some(given)) => given,
            ("conflict", None) => {
                return Err(Error::field(
                    "accept",
                    format!("{measure}: {} is already defined here, differently; import it under another name or leave it", proposals[i].name),
                ));
            }
            (_, _) => {
                return Err(Error::field(
                    "accept",
                    format!("{measure} is not imported ({status}): {}", review[i]["reason"].as_str().unwrap_or("nothing to import")),
                ));
            }
        };
        if !names.insert(name.to_lowercase()) {
            return Err(Error::field("accept", format!("{measure}: there is already a metric called {name}")));
        }
        creating.push((i, name));
    }
    let mut created = Vec::new();
    if !creating.is_empty() {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        for (i, name) in &creating {
            let p = &proposals[*i];
            let Ok((spec, series_by)) = &p.definition else { continue };
            let doc = obj(json!({"metric_name": name, "description": p.description, "definition": spec, "series_by": series_by}));
            let made = k.insert_tx(ctx, &mut tx, "Metric", doc).await?;
            review[*i]["created"] = json!(id(&made, "id"));
            created.push(json!({"measure": p.measure, "metric_name": name, "id": id(&made, "id"), "status": s(&made, "status")}));
        }
        tx.commit().await?;
    }
    Ok(json!({"review": review, "created": created}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_aggregate_of_one_column_is_brought() {
        let a = |e: &str| aggregate(e, "sales");
        assert_eq!(a("SUM(amount)"), Some((Agg::Sum, Some("amount".into()))));
        assert_eq!(a(" sum( \"amount\" ) "), Some((Agg::Sum, Some("amount".into()))));
        assert_eq!(a("sum(sales.amount)"), Some((Agg::Sum, Some("amount".into()))));
        assert_eq!(a("count(*)"), Some((Agg::Count, None)));
        assert_eq!(a("count(customer)"), Some((Agg::Count, Some("customer".into()))));
        assert_eq!(a("COUNT(DISTINCT customer)"), Some((Agg::CountDistinct, Some("customer".into()))));
        assert_eq!(a("avg(amount)").map(|x| x.0), Some(Agg::Avg));
        assert_eq!(a("max(amount)").map(|x| x.0), Some(Agg::Max));
        // Anything else has no exact meaning: never approximated.
        for e in [
            "sum(amount * 2)",
            "sum(amount) / count(*)",
            "sum(case when status = 'Paid' then amount end)",
            "sum(coalesce(amount, 0))",
            "sum(orders.amount)",
            "median(amount)",
            "sum(distinct amount)",
            "count(distinct *)",
            "sum()",
            "amount",
            "sum(amount",
        ] {
            assert_eq!(a(e), None, "{e}");
        }
    }
}
