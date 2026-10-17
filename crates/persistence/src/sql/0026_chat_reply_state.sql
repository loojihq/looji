-- How an assistant reply ended. A reply that was stopped or failed keeps its text, and a failed one keeps the reason.
-- Messages written before this are complete.
ALTER TABLE task_messages ADD COLUMN state TEXT NOT NULL DEFAULT 'complete'
    CHECK (state IN ('complete', 'stopped', 'failed'));
ALTER TABLE task_messages ADD COLUMN error TEXT;
