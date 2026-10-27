-- PDT-174: a project's settings for each integration, as one JSON object keyed by the integration's id
-- ({"jira": {"connection": …, "scope": …, "done": …}, "buildkite": {"connection": …, "pipelines": […]}}), each
-- integration declaring its fields. The Jira and Buildkite columns 0027 added are copied in and left unused rather
-- than dropped: SQLite's DROP COLUMN rewrites the whole table, and empty columns cost nothing.

ALTER TABLE project_settings ADD COLUMN integration_settings TEXT NOT NULL DEFAULT '{}';

UPDATE project_settings
SET integration_settings = json_set(integration_settings, '$.jira',
    json_object('connection', jira_connection, 'scope', jira_scope, 'done', jira_done))
WHERE jira_connection IS NOT NULL AND jira_connection != '';

UPDATE project_settings
SET integration_settings = json_set(integration_settings, '$.buildkite',
    json_object('connection', buildkite_connection, 'pipelines',
        CASE WHEN json_valid(buildkite_pipelines) THEN json(buildkite_pipelines) ELSE json('[]') END))
WHERE buildkite_connection IS NOT NULL AND buildkite_connection != '';
