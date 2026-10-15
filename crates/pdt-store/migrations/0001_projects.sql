CREATE TABLE projects (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    repo_path   TEXT NOT NULL UNIQUE,
    base_branch TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
