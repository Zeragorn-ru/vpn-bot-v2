ALTER TABLE notifications ADD COLUMN IF NOT EXISTS source_event_id uuid REFERENCES outbox_events(id);
CREATE UNIQUE INDEX IF NOT EXISTS notifications_source_event_idx ON notifications (source_event_id) WHERE source_event_id IS NOT NULL;
