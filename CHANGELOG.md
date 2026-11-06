# Changelog

## 0.2.1

Rebuilt with Ferrum-Frontend bb60115: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.2.0

Followed metrics (FERRUM-ECOSYSTEM-010: scheduled delivery).

- A person follows a certified metric (`MetricSubscription`), daily, weekly or monthly, for
  themselves only, once each. On its day in the workspace, upkeep reads it with their own rights,
  keeps the value as theirs (a Metric Value), and tells them by `metrics.digest`, in the app unless
  they chose otherwise. Each follower sees only their own subscriptions.
- A metric deprecated since is said so once, naming what replaces it, and stops. One back in draft
  keeps why it was not sent, and is delivered again once certified. A follower no longer active
  keeps why too.
- Personal-data requests now reach metrics' owners and certifiers, terms' owners, readers of
  values and followers.

## 0.1.3

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.1.2

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.1.1

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## 0.1.0

Governed metrics on Studio (FERRUM-ECOSYSTEM-010).

- A metric as a Studio query of one measure, checked when saved; its lineage worked out from it.
- Certification by a Data Steward who does not own it; a changed definition back to draft.
- Values read with each person's own rights, at `/api/app/metrics/value`.
- Values kept as snapshots, with the definition's fingerprint.
- Deprecation naming the replacement; a glossary of business terms.
- Its own interface at `/kpis/`: the certified metrics by domain.
