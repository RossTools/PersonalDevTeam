-- MCP servers reach agents (PDT-322) unless their `config.agents.enabled` is false. Servers added before then were
-- added for workflows only: they stay so until the user turns Agents on in Settings › MCP servers.
UPDATE connections
SET config = json_set(CASE WHEN json_valid(config) THEN config ELSE '{}' END, '$.agents', json('{"enabled": false}'))
WHERE kind = 'mcp' AND (NOT json_valid(config) OR json_extract(config, '$.agents') IS NULL);
