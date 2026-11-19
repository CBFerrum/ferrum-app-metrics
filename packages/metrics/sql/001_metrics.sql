-- One active follow per person and metric, held by the database, not only by the save's own check (which two saves
-- at once could both pass). Duplicates such a race let in before are stopped first, the earliest kept, so the index
-- builds on every workspace.
update e_metric_subscription s
   set active = false, last_error = 'Followed twice: this one was stopped, the first is kept'
 where s.active
   and exists (select 1 from e_metric_subscription o
                 where o.tenant_id = s.tenant_id and o.active and o.metric = s.metric
                   and lower(o.subscriber::text) = lower(s.subscriber::text) and o.id < s.id);
create unique index if not exists metrics_follow_once
    on e_metric_subscription (tenant_id, lower(subscriber::text), metric) where active;
