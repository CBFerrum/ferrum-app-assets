-- The per-minute upkeep reads only what is due, never the history: schedule rows not yet booked,
-- and maintenance tasks by their next due date.
create index if not exists ix_asset_schedule_unbooked on e_asset__schedule (tenant_id, schedule_date) where not booked;
create index if not exists ix_asset_maintenance_task_due on e_asset_maintenance__tasks (tenant_id, next_due_date);
