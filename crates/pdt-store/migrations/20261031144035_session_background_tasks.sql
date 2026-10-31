-- A conversation's background tasks (PDT-311), as JSON arrays of `BackgroundTask`: the ones running as last reported,
-- and the ones a pdtd restart stopped that the agent has not been told about yet.
ALTER TABLE sessions ADD COLUMN background_tasks TEXT;
ALTER TABLE sessions ADD COLUMN stopped_background TEXT;
