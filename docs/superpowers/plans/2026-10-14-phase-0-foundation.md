# Phase 0 — Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A running `pdtd` service (Rust, SQLite, REST API) that serves the Graphite-themed web interface frame, with a project registry, generated TypeScript types, a Tauri desktop window that starts the service or attaches to one already running, and CI on Windows and macOS.

**Architecture:** A Cargo workspace of small crates:
- `pdt-core`: pure types and naming constants.
- `pdt-git`: calls the git command line.
- `pdt-store`: SQLite with migrations and a backup before each migration.
- `pdt-api`: axum routes plus static file serving for the single-page app.
- `pdtd`: the service program.

The React/Vite interface lives in `web/`, with the design system in `web/ui/`. The desktop app (`desktop/src-tauri`) is a thin Tauri v2 window pointed at the service's URL. TypeScript API types are generated from Rust by ts-rs.

**Tech stack:**
- Rust 1.99 (edition 2024): tokio 1.53, axum 0.8.9, tower-http 0.7, sqlx 0.9 (SQLite), ts-rs 12, jiff 0.2, uuid 1.28
- Node 24 LTS, pnpm 12
- React 19.3, TypeScript 7, Vite 8, Tailwind 4.3, Vitest 5
- Tauri 2.12

**Spec:** `docs/superpowers/specs/2026-10-14-personal-dev-team-design.md` (§3, §7.2, §8, §9, §11 Phase 0)

## Global Constraints

- Rust toolchain is pinned to `1.99.0`, edition `2024`, workspace resolver `3`.
- Pin at least these versions:
  - tokio 1.53.3, axum 0.8.9, tower 0.5.3, tower-http 0.7.1, sqlx 0.9.0
  - serde 1.0.229, serde_json 1.0.154, ts-rs 12.0.1
  - thiserror 2.0.21, anyhow 1.0.104, tracing 0.1.44, tracing-subscriber 0.3.23
  - uuid 1.28.0, jiff 0.2.38
  - tauri 2.12.4, tauri-build 2.7.3
- Pin at least these JS versions:
  - react / react-dom 19.3.1, typescript 7.0.2, vite 8.3.4, @vitejs/plugin-react 6.1.2
  - @tanstack/react-query 5.104.2, tailwindcss 4.3.3, vitest 5.0.4
  - @tauri-apps/cli 2.12.3, Node 24 LTS, pnpm 12.11.2
- Dependencies not listed in the spec are added with `cargo add` or `pnpm add` so they resolve to the latest version on the day you add them. Never type a version from memory.
- Library crates use `thiserror`. Only the `pdtd` program uses `anyhow`. API errors are always JSON in the form `{ "code": string, "message": string }`.
- `pdtd` listens only on `127.0.0.1`.
- Names are defined in exactly one place, `crates/pdt-core/src/brand.rs`:
  - product name `Personal Dev Team`, slug `pdt`, service `pdtd`, stable port `7420`, dev port `7421`.
  - The only exception is `desktop/src-tauri/tauri.conf.json`, which has to repeat the product name and identifier. A comment in `brand.rs` points to it.
- Profiles: `PDT_PROFILE=dev` selects the dev port and the data folder `<data_dir>/pdt-dev`. Anything else selects stable (`<data_dir>/pdt`).
- Colors in the interface come only from tokens in `web/ui/tokens.css`. Components never contain raw hex values.
  - **Dark mode:** bg `#0e0f12`, surface `#17181d`, border `#26282f`, fg `#e7e8ec`, muted `#8a8f9c`, accent `#3561b8`, text on accent `#eef2fb`.
  - **Light mode:** bg `#f7f7f9`, accent `#2563eb`.
  - **Fonts:** Inter for the interface, JetBrains Mono for code. Corner radius 6px.
- Timestamps are `i64` Unix milliseconds, exported to TypeScript as `number`.
- Everything must build and pass tests on both Windows and macOS.

## Review Focus

1. **Repo paths with spaces, non-ASCII characters or Windows separators** (`C:\Users\me\my repo é`) must register correctly, and the path returned is the repo root. Pinned in Task 3 (`resolves_path_with_spaces_and_unicode`).
2. **Registering the same repo twice,** including through a subdirectory or a different path spelling, returns `409 conflict`, not `500`. Pinned in Task 5 (`duplicate_repo_is_conflict`).
3. **A path that doesn't exist, or a folder that isn't a git repo,** returns `400` with a specific `code` (`path_not_found` / `not_a_git_repo`), not a crash. Pinned in Tasks 3 and 5.
4. **Opening an existing database whose schema is out of date** first makes a backup copy, then migrates. A brand-new database makes no backup. Pinned in Task 4 (`backs_up_before_pending_migration`, `fresh_db_makes_no_backup`).
5. **Opening the desktop app while `pdtd` is already running** attaches to that service and does not start a second one. If the port is taken, `pdtd` exits with a clear message instead of panicking. Pinned in Task 9 (`does_not_spawn_when_already_listening`) and Task 6 (`port_in_use_is_reported`).

---

## File Structure

```
rust-toolchain.toml                 toolchain pin
Cargo.toml                          workspace + shared dependency versions
.cargo/config.toml                  TS_RS_EXPORT_DIR
CLAUDE.md                           working guide for agents building PDT (self-hosting)
package.json, pnpm-workspace.yaml   JS workspace (web, desktop)
crates/pdt-core/src/{lib,brand,project}.rs
crates/pdt-git/src/lib.rs           resolve_repo (git CLI)
crates/pdt-store/src/{lib,error,migrate}.rs, migrations/0001_projects.sql
crates/pdt-api/src/{lib,error,routes}.rs, tests/api.rs
crates/pdtd/src/{main,config}.rs
web/                                Vite app
  ui/tokens.css, ui/theme.ts, ui/Button.tsx, ui/StatusDot.tsx, ui/index.ts
  src/main.tsx, src/index.css, src/api/client.ts, src/api/gen/*.ts (generated)
  src/shell/{AppShell,IconRail,BottomPanel,views}.tsx, src/projects/{ProjectsList,AddProjectForm}.tsx
desktop/package.json, desktop/app-icon.svg
desktop/src-tauri/{Cargo.toml,build.rs,tauri.conf.json,src/main.rs,src/daemon.rs}
.github/workflows/ci.yml
```

---

### Task 1: Toolchain, workspace skeleton and repo guide

**Files:**
- Create: `rust-toolchain.toml`, `Cargo.toml`, `.cargo/config.toml`, `CLAUDE.md`, `crates/pdt-core/Cargo.toml`, `crates/pdt-core/src/lib.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces: a workspace with shared dependencies (`[workspace.dependencies]`) that every later crate uses through `dep.workspace = true`.

- [ ] **Step 1: Install the toolchain (Windows dev machine; on macOS use `brew install rustup` and skip winget)**

```powershell
winget install --id Rustlang.Rustup -e
rustup toolchain install 1.99.0 --component rustfmt clippy
npm install -g pnpm@12.11.2
npm install -g @openai/codex@latest
```

Check: `rustc +1.99.0 --version` prints `rustc 1.99.0`, `pnpm --version` prints `12.11.2`, and `codex --version` prints `0.162.1` or later.

If linking fails later with `link.exe not found`, install the MSVC build tools with `winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --quiet"`.

- [ ] **Step 2: Write the workspace files**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "1.99.0"
components = ["rustfmt", "clippy"]
```

`Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.99"
publish = false

[workspace.dependencies]
pdt-core = { path = "crates/pdt-core" }
pdt-git = { path = "crates/pdt-git" }
pdt-store = { path = "crates/pdt-store" }
pdt-api = { path = "crates/pdt-api" }

tokio = { version = "1.53.3", features = ["full"] }
axum = "0.8.9"
tower = { version = "0.5.3", features = ["util"] }
tower-http = { version = "0.7.1", features = ["fs", "trace"] }
sqlx = { version = "0.9.0", default-features = false, features = ["sqlite", "runtime-tokio", "migrate", "macros"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
ts-rs = "12.0.1"
thiserror = "2.0.21"
anyhow = "1.0.104"
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
uuid = { version = "1.28.0", features = ["v7"] }
jiff = "0.2.38"
```

`.cargo/config.toml`:
```toml
[env]
TS_RS_EXPORT_DIR = { value = "web/src/api/gen", relative = true }
```

`crates/pdt-core/Cargo.toml`:
```toml
[package]
name = "pdt-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[dependencies]
serde.workspace = true
ts-rs.workspace = true
thiserror.workspace = true
```

`crates/pdt-core/src/lib.rs`:
```rust
//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {}
}
```

Append to `.gitignore`:
```
*.db
```

- [ ] **Step 3: Write `CLAUDE.md` (the guide agents will read while building PDT)**

```markdown
# Personal Dev Team (PDT)

Personal command center for agentic development: tickets + pipeline board, Claude Code / Codex sessions, review, terminals.
Spec: docs/superpowers/specs/2026-10-14-personal-dev-team-design.md. Plans: docs/superpowers/plans/.

## Layout
- crates/pdt-core — pure domain types + brand constants (no I/O)
- crates/pdt-git — git CLI wrapper
- crates/pdt-store — SQLite (sqlx) + migrations in crates/pdt-store/migrations
- crates/pdt-api — axum routes; errors are JSON {code, message}
- crates/pdtd — daemon binary
- web/ — React + Vite UI; web/ui is the design system (tokens.css is the only place colors live)
- desktop/src-tauri — Tauri v2 shell

## Commands
- Rust: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`
- TS types are generated by `cargo test` into web/src/api/gen — commit them.
- Web: `pnpm --dir web dev | test | typecheck | build`
- Run a dev daemon: `PDT_PROFILE=dev cargo run -p pdtd` (port 7421, data dir pdt-dev)

## Rules
- TDD for core logic. Tests must pass on Windows and macOS.
- Never run or restart the *stable* pdtd (port 7420) from a dev build — it hosts the agents building this repo. Use PDT_PROFILE=dev.
- Names (product, slug, daemon, ports) live only in crates/pdt-core/src/brand.rs (+ tauri.conf.json).
- New dependencies: add with `cargo add` / `pnpm add` (latest), never versions from memory.
```

- [ ] **Step 4: Check that the workspace builds and tests pass**

Run: `cargo test`
Expected: `test tests::workspace_builds ... ok`

- [ ] **Step 5: Commit**

```bash
git add rust-toolchain.toml Cargo.toml .cargo/config.toml CLAUDE.md crates/pdt-core .gitignore Cargo.lock
git commit -m "chore: cargo workspace skeleton, toolchain pin, agent guide"
```

---

### Task 2: `pdt-core` naming constants, profiles and Project types

**Files:**
- Create: `crates/pdt-core/src/brand.rs`, `crates/pdt-core/src/project.rs`
- Modify: `crates/pdt-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `pdt_core::brand::{PRODUCT_NAME, SLUG, DAEMON_BIN, STABLE_PORT, DEV_PORT, Profile}`
  - `Profile::from_env_value(Option<&str>) -> Profile`, `Profile::port(self) -> u16`, `Profile::data_dir_name(self) -> String`
  - `pdt_core::project::{Project, NewProject, ValidProject, ValidationError}`
  - `NewProject::validate(&self) -> Result<ValidProject, ValidationError>`
  - Generated TypeScript: `web/src/api/gen/Project.ts`, `NewProject.ts`

- [ ] **Step 1: Write the failing tests**

`crates/pdt-core/src/brand.rs`:
```rust
//! The single source of product naming. A rename touches this file and
//! desktop/src-tauri/tauri.conf.json (productName, identifier) only.

pub const PRODUCT_NAME: &str = "Personal Dev Team";
pub const SLUG: &str = "pdt";
pub const DAEMON_BIN: &str = "pdtd";
pub const STABLE_PORT: u16 = 7420;
pub const DEV_PORT: u16 = 7421;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_profile_only_from_exact_value() {
        assert_eq!(Profile::from_env_value(Some("dev")), Profile::Dev);
        assert_eq!(Profile::from_env_value(Some("DEV")), Profile::Stable);
        assert_eq!(Profile::from_env_value(Some("")), Profile::Stable);
        assert_eq!(Profile::from_env_value(None), Profile::Stable);
    }

    #[test]
    fn profiles_have_distinct_ports_and_dirs() {
        assert_eq!(Profile::Stable.port(), 7420);
        assert_eq!(Profile::Dev.port(), 7421);
        assert_eq!(Profile::Stable.data_dir_name(), "pdt");
        assert_eq!(Profile::Dev.data_dir_name(), "pdt-dev");
    }
}
```

`crates/pdt-core/src/project.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, path: &str, branch: Option<&str>) -> NewProject {
        NewProject { name: name.into(), repo_path: path.into(), base_branch: branch.map(Into::into) }
    }

    #[test]
    fn trims_fields() {
        let v = input("  PDT ", " C:\\repo ", Some(" main ")).validate().unwrap();
        assert_eq!(v, ValidProject { name: "PDT".into(), repo_path: "C:\\repo".into(), base_branch: Some("main".into()) });
    }

    #[test]
    fn rejects_blank_name_and_path() {
        assert_eq!(input("  ", "/r", None).validate(), Err(ValidationError::EmptyName));
        assert_eq!(input("x", "   ", None).validate(), Err(ValidationError::EmptyPath));
    }

    #[test]
    fn blank_branch_is_error_but_missing_branch_is_fine() {
        assert_eq!(input("x", "/r", Some(" ")).validate(), Err(ValidationError::EmptyBranch));
        assert_eq!(input("x", "/r", None).validate().unwrap().base_branch, None);
    }
}
```

`crates/pdt-core/src/lib.rs`:
```rust
//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

pub mod brand;
pub mod project;
```

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p pdt-core`
Expected: compile errors: `cannot find type Profile`, `cannot find type NewProject`.

- [ ] **Step 3: Implement**

Add to `crates/pdt-core/src/brand.rs`, above the tests:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Stable,
    Dev,
}

impl Profile {
    /// Interprets the `PDT_PROFILE` environment value. Only the exact string `dev` selects Dev.
    pub fn from_env_value(value: Option<&str>) -> Profile {
        match value {
            Some("dev") => Profile::Dev,
            _ => Profile::Stable,
        }
    }

    pub fn port(self) -> u16 {
        match self {
            Profile::Stable => STABLE_PORT,
            Profile::Dev => DEV_PORT,
        }
    }

    pub fn data_dir_name(self) -> String {
        match self {
            Profile::Stable => SLUG.to_string(),
            Profile::Dev => format!("{SLUG}-dev"),
        }
    }
}
```

Add to `crates/pdt-core/src/project.rs`, above the tests:
```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Absolute repo root, as reported by git, using native separators.
    pub repo_path: String,
    pub base_branch: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewProject {
    pub name: String,
    pub repo_path: String,
    #[serde(default)]
    #[ts(optional)]
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidProject {
    pub name: String,
    pub repo_path: String,
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("project name must not be empty")]
    EmptyName,
    #[error("repository path must not be empty")]
    EmptyPath,
    #[error("base branch must not be blank")]
    EmptyBranch,
}

impl NewProject {
    pub fn validate(&self) -> Result<ValidProject, ValidationError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        let repo_path = self.repo_path.trim();
        if repo_path.is_empty() {
            return Err(ValidationError::EmptyPath);
        }
        let base_branch = match self.base_branch.as_deref().map(str::trim) {
            Some("") => return Err(ValidationError::EmptyBranch),
            Some(b) => Some(b.to_string()),
            None => None,
        };
        Ok(ValidProject { name: name.into(), repo_path: repo_path.into(), base_branch })
    }
}
```

- [ ] **Step 4: Run the tests and check that they pass and the TypeScript files are generated**

Run: `cargo test -p pdt-core`
Expected: 5 tests pass, and the files `web/src/api/gen/Project.ts` and `web/src/api/gen/NewProject.ts` exist. `Project.ts` contains `createdAt: number` and `NewProject.ts` contains `baseBranch?: string`.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-core web/src/api/gen
git commit -m "feat(core): brand constants, profiles, project types with TS export"
```

---

### Task 3: `pdt-git` repo resolution

**Files:**
- Create: `crates/pdt-git/Cargo.toml`, `crates/pdt-git/src/lib.rs`

**Interfaces:**
- Produces:
  - `pdt_git::resolve_repo(path: &Path) -> Result<RepoInfo, GitError>` (blocking)
  - `RepoInfo { root: PathBuf, current_branch: Option<String> }`
  - `GitError::{PathNotFound(PathBuf), NotARepo(PathBuf), GitMissing, Command(String)}`

- [ ] **Step 1: Create the crate and write the failing tests**

`crates/pdt-git/Cargo.toml`:
```toml
[package]
name = "pdt-git"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[dependencies]
thiserror.workspace = true
```

Then run: `cargo add --dev tempfile -p pdt-git`

`crates/pdt-git/src/lib.rs`:
```rust
//! Thin wrapper over the `git` CLI. Blocking; call from `spawn_blocking` in async code.

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo(dir: &Path) {
        let ok = Command::new("git").args(["init", "-q", "-b", "main"]).arg(dir).status().unwrap().success();
        assert!(ok, "git init failed");
    }

    fn same_dir(a: &Path, b: &Path) -> bool {
        std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
    }

    #[test]
    fn resolves_root_from_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let sub = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        let info = resolve_repo(&sub).unwrap();
        assert!(same_dir(&info.root, tmp.path()));
        assert_eq!(info.current_branch.as_deref(), Some("main"));
    }

    #[test]
    fn resolves_path_with_spaces_and_unicode() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("my repo é");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let info = resolve_repo(&repo).unwrap();
        assert!(same_dir(&info.root, &repo));
        assert!(info.root.to_string_lossy().ends_with("my repo é"));
        if cfg!(windows) {
            assert!(!info.root.to_string_lossy().contains('/'), "expected native separators");
        }
    }

    #[test]
    fn plain_directory_is_not_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(resolve_repo(tmp.path()), Err(GitError::NotARepo(_))));
    }

    #[test]
    fn missing_path_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        assert!(matches!(resolve_repo(&missing), Err(GitError::PathNotFound(_))));
    }
}
```

The `crates/*` member glob already includes the new crate.

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p pdt-git`
Expected: compile error: `cannot find function resolve_repo`.

- [ ] **Step 3: Implement** (above the tests in `lib.rs`)

```rust
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub current_branch: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("path does not exist or is not a directory: {0}")]
    PathNotFound(PathBuf),
    #[error("not a git repository: {0}")]
    NotARepo(PathBuf),
    #[error("git is not installed or not on PATH")]
    GitMissing,
    #[error("git command failed: {0}")]
    Command(String),
}

/// Runs `git -C dir args…`. Ok(None) means git ran but exited non-zero.
fn git(dir: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| {
        if e.kind() == ErrorKind::NotFound { GitError::GitMissing } else { GitError::Command(e.to_string()) }
    })?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_string()))
    } else {
        Ok(None)
    }
}

fn native_separators(s: String) -> String {
    if cfg!(windows) { s.replace('/', "\\") } else { s }
}

/// Resolves any path inside a working tree to the repo root and its current branch.
pub fn resolve_repo(path: &Path) -> Result<RepoInfo, GitError> {
    if !path.is_dir() {
        return Err(GitError::PathNotFound(path.to_path_buf()));
    }
    let root = git(path, &["rev-parse", "--show-toplevel"])?
        .filter(|s| !s.is_empty())
        .ok_or_else(|| GitError::NotARepo(path.to_path_buf()))?;
    let root = PathBuf::from(native_separators(root));
    let current_branch = git(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?.filter(|s| !s.is_empty());
    Ok(RepoInfo { root, current_branch })
}
```

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo test -p pdt-git`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-git Cargo.lock
git commit -m "feat(git): resolve repo root and current branch via git CLI"
```

---

### Task 4: `pdt-store` SQLite, migrations with backup, and project storage

**Files:**
- Create: `crates/pdt-store/Cargo.toml`, `crates/pdt-store/src/lib.rs`, `crates/pdt-store/src/error.rs`, `crates/pdt-store/src/migrate.rs`, `crates/pdt-store/migrations/0001_projects.sql`

**Interfaces:**
- Consumes: `pdt_core::project::Project`
- Produces:
  - `pdt_store::Store` (Clone)
  - `Store::open(db_path: &Path) -> Result<Store, StoreError>`
  - `Store::open_with(db_path: &Path, migrator: &Migrator, backup_dir: &Path) -> Result<Store, StoreError>`
  - `Store::insert_project(&self, &Project) -> Result<(), StoreError>`
  - `Store::list_projects(&self) -> Result<Vec<Project>, StoreError>`
  - `Store::get_project(&self, id: &str) -> Result<Option<Project>, StoreError>`
  - `Store::close(self)`
  - `StoreError::{Conflict(String), Db(sqlx::Error), Migrate(sqlx::migrate::MigrateError), Io(std::io::Error)}`

- [ ] **Step 1: Create the crate and migration, and write the failing tests**

`crates/pdt-store/Cargo.toml`:
```toml
[package]
name = "pdt-store"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[dependencies]
pdt-core.workspace = true
sqlx.workspace = true
thiserror.workspace = true
jiff.workspace = true

[dev-dependencies]
tokio.workspace = true
```

Then run: `cargo add --dev tempfile -p pdt-store`

`crates/pdt-store/migrations/0001_projects.sql`:
```sql
CREATE TABLE projects (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    repo_path   TEXT NOT NULL UNIQUE,
    base_branch TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
```

`crates/pdt-store/src/error.rs`:
```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Db(sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e {
            if db.is_unique_violation() {
                return StoreError::Conflict(db.message().to_string());
            }
        }
        StoreError::Db(e)
    }
}
```

`crates/pdt-store/src/lib.rs` (tests first; the implementation is added in Step 3):
```rust
mod error;
mod migrate;

pub use error::StoreError;

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::project::Project;

    fn project(id: &str, path: &str) -> Project {
        Project { id: id.into(), name: "P".into(), repo_path: path.into(), base_branch: "main".into(), created_at: 1 }
    }

    #[tokio::test]
    async fn insert_list_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let p = project("a", "C:\\work\\my repo é");
        store.insert_project(&p).await.unwrap();
        assert_eq!(store.list_projects().await.unwrap(), vec![p.clone()]);
        assert_eq!(store.get_project("a").await.unwrap(), Some(p));
        assert_eq!(store.get_project("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn duplicate_repo_path_is_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        store.insert_project(&project("a", "/r")).await.unwrap();
        let err = store.insert_project(&project("b", "/r")).await.unwrap_err();
        assert!(matches!(err, StoreError::Conflict(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn data_survives_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("pdt.db");
        let store = Store::open(&db).await.unwrap();
        store.insert_project(&project("a", "/r")).await.unwrap();
        store.close().await;
        let store = Store::open(&db).await.unwrap();
        assert_eq!(store.list_projects().await.unwrap().len(), 1);
    }
}
```

`crates/pdt-store/src/migrate.rs` (tests first):
```rust
#[cfg(test)]
mod tests {
    use crate::Store;
    use sqlx::migrate::Migrator;
    use std::path::Path;

    fn write(dir: &Path, name: &str, sql: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), sql).unwrap();
    }

    fn backups(dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|r| r.count()).unwrap_or(0)
    }

    #[tokio::test]
    async fn fresh_db_makes_no_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let mig = tmp.path().join("m1");
        write(&mig, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        let migrator = Migrator::new(mig.as_path()).await.unwrap();
        let backup_dir = tmp.path().join("backups");
        let s = Store::open_with(&tmp.path().join("t.db"), &migrator, &backup_dir).await.unwrap();
        s.close().await;
        assert_eq!(backups(&backup_dir), 0);
    }

    #[tokio::test]
    async fn backs_up_before_pending_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let backup_dir = tmp.path().join("backups");
        let m1 = tmp.path().join("m1");
        write(&m1, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        let m2 = tmp.path().join("m2");
        write(&m2, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        write(&m2, "0002_b.sql", "CREATE TABLE b (y INTEGER);");

        let s = Store::open_with(&db, &Migrator::new(m1.as_path()).await.unwrap(), &backup_dir).await.unwrap();
        s.close().await;
        assert_eq!(backups(&backup_dir), 0, "no pending migration on reopen with same set");

        let s = Store::open_with(&db, &Migrator::new(m2.as_path()).await.unwrap(), &backup_dir).await.unwrap();
        s.close().await;
        assert_eq!(backups(&backup_dir), 1, "one backup before applying 0002");
    }
}
```

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p pdt-store`
Expected: compile errors: `cannot find type Store`.

- [ ] **Step 3: Implement**

`crates/pdt-store/src/migrate.rs`, above the tests:
```rust
use crate::StoreError;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use std::collections::HashSet;
use std::path::Path;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn applied_versions(pool: &SqlitePool) -> Result<HashSet<i64>, StoreError> {
    let table: Option<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'")
            .fetch_optional(pool)
            .await?;
    if table.is_none() {
        return Ok(HashSet::new());
    }
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT version FROM _sqlx_migrations WHERE success = 1")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

/// Copies the database (VACUUM INTO) when it already has applied migrations
/// and the migrator would apply more. Fresh databases are not backed up.
pub async fn backup_if_pending(pool: &SqlitePool, migrator: &Migrator, backup_dir: &Path) -> Result<(), StoreError> {
    let applied = applied_versions(pool).await?;
    let pending = migrator.iter().any(|m| !applied.contains(&m.version));
    if applied.is_empty() || !pending {
        return Ok(());
    }
    std::fs::create_dir_all(backup_dir)?;
    let file = backup_dir.join(format!("pdt-{}.db", jiff::Timestamp::now().as_millisecond()));
    let target = file.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{target}'")).execute(pool).await?;
    Ok(())
}
```

`crates/pdt-store/src/lib.rs`, replacing the module header above the tests:
```rust
mod error;
mod migrate;

pub use error::StoreError;

use pdt_core::project::Project;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::Path;

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

type ProjectRow = (String, String, String, String, i64);

fn to_project((id, name, repo_path, base_branch, created_at): ProjectRow) -> Project {
    Project { id, name, repo_path, base_branch, created_at }
}

impl Store {
    /// Opens (creating if needed) the database, backing up to `<db dir>/backups`
    /// before applying any pending migrations.
    pub async fn open(db_path: &Path) -> Result<Store, StoreError> {
        let backup_dir = db_path.parent().unwrap_or(Path::new(".")).join("backups");
        Self::open_with(db_path, &migrate::MIGRATOR, &backup_dir).await
    }

    pub async fn open_with(db_path: &Path, migrator: &Migrator, backup_dir: &Path) -> Result<Store, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
        migrate::backup_if_pending(&pool, migrator, backup_dir).await?;
        migrator.run(&pool).await?;
        Ok(Store { pool })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    pub async fn insert_project(&self, p: &Project) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO projects (id, name, repo_path, base_branch, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&p.id)
            .bind(&p.name)
            .bind(&p.repo_path)
            .bind(&p.base_branch)
            .bind(p.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        let rows: Vec<ProjectRow> = sqlx::query_as(
            "SELECT id, name, repo_path, base_branch, created_at FROM projects ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(to_project).collect())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<Project>, StoreError> {
        let row: Option<ProjectRow> =
            sqlx::query_as("SELECT id, name, repo_path, base_branch, created_at FROM projects WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(to_project))
    }
}
```

If sqlx 0.9 renamed any of `SqliteConnectOptions::filename`, `journal_mode`, `Migrator::iter`, `Migrator::new` or `is_unique_violation`, check docs.rs/sqlx/0.9.0 for the new name and update the plan's later references to match. The behavior asserted by the tests must not change.

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo test -p pdt-store`
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-store Cargo.lock
git commit -m "feat(store): sqlite store with WAL, migrations, pre-migration backup, projects"
```

---

### Task 5: `pdt-api` routes, JSON errors and single-page-app serving

**Files:**
- Create: `crates/pdt-api/Cargo.toml`, `crates/pdt-api/src/lib.rs`, `crates/pdt-api/src/error.rs`, `crates/pdt-api/src/routes.rs`, `crates/pdt-api/tests/api.rs`

**Interfaces:**
- Consumes: `Store` (Task 4), `pdt_git::resolve_repo` (Task 3), `NewProject::validate` (Task 2), `brand::PRODUCT_NAME`
- Produces:
  - `pdt_api::AppState { pub store: Store }` (Clone)
  - `pdt_api::router(state: AppState, web_dir: Option<PathBuf>) -> axum::Router`
  - `pdt_api::{Health, ApiErrorBody}` (ts-rs exported)
  - HTTP endpoints:
    - `GET /api/health` → `Health`
    - `GET /api/projects` → `Project[]`
    - `POST /api/projects` (`NewProject`) → `201 Project`
    - `GET /api/projects/{id}` → `Project` or 404
    - unknown `/api/*` → 404 JSON
    - other paths → files from `web_dir`, falling back to `index.html`
  - Error codes: `invalid_input`, `path_not_found`, `not_a_git_repo`, `git_missing`, `conflict`, `not_found`, `internal`

- [ ] **Step 1: Create the crate and write the failing integration tests**

`crates/pdt-api/Cargo.toml`:
```toml
[package]
name = "pdt-api"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[dependencies]
pdt-core.workspace = true
pdt-git.workspace = true
pdt-store.workspace = true
axum.workspace = true
tower-http.workspace = true
tokio.workspace = true
serde.workspace = true
ts-rs.workspace = true
tracing.workspace = true
uuid.workspace = true
jiff.workspace = true

[dev-dependencies]
tower.workspace = true
serde_json.workspace = true
```

Then run: `cargo add --dev tempfile http-body-util -p pdt-api`

`crates/pdt-api/tests/api.rs`:
```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use tower::ServiceExt;

async fn app(tmp: &Path, web_dir: Option<std::path::PathBuf>) -> axum::Router {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    router(AppState { store }, web_dir)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(dir).status().unwrap().success());
}

#[tokio::test]
async fn health_reports_product() {
    let tmp = tempfile::tempdir().unwrap();
    let (status, body) = call(&app(tmp.path(), None).await, "GET", "/api/health", None).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["name"], "Personal Dev Team");
}

#[tokio::test]
async fn create_then_list_and_get_project() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("my repo é");
    init_repo(&repo);
    let app = app(tmp.path(), None).await;

    let (status, body) =
        call(&app, "POST", "/api/projects", Some(json!({"name": " PDT ", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(created["name"], "PDT");
    assert_eq!(created["baseBranch"], "main");
    assert!(created["repoPath"].as_str().unwrap().ends_with("my repo é"));

    let (_, list) = call(&app, "GET", "/api/projects", None).await;
    assert_eq!(serde_json::from_str::<Value>(&list).unwrap().as_array().unwrap().len(), 1);

    let id = created["id"].as_str().unwrap();
    let (status, _) = call(&app, "GET", &format!("/api/projects/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&app, "GET", "/api/projects/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "not_found");
}

#[tokio::test]
async fn duplicate_repo_is_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("r");
    init_repo(&repo);
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    let app = app(tmp.path(), None).await;
    let (s1, _) = call(&app, "POST", "/api/projects", Some(json!({"name": "a", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, body) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "b", "repoPath": repo.join("sub").to_string_lossy()})))
            .await;
    assert_eq!(s2, StatusCode::CONFLICT, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "conflict");
}

#[tokio::test]
async fn bad_paths_and_input_are_400_with_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let app = app(tmp.path(), None).await;
    for (body, code) in [
        (json!({"name": "x", "repoPath": plain.to_string_lossy()}), "not_a_git_repo"),
        (json!({"name": "x", "repoPath": tmp.path().join("missing").to_string_lossy()}), "path_not_found"),
        (json!({"name": "  ", "repoPath": plain.to_string_lossy()}), "invalid_input"),
    ] {
        let (status, resp) = call(&app, "POST", "/api/projects", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["code"], code);
        assert!(!v["message"].as_str().unwrap().is_empty());
    }
}

#[tokio::test]
async fn unknown_api_route_is_json_404_and_spa_falls_back_to_index() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>PDT</html>").unwrap();
    let app = app(tmp.path(), Some(web)).await;

    let (status, body) = call(&app, "GET", "/api/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "not_found");

    let (status, body) = call(&app, "GET", "/board/deep/link", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("PDT"));
}
```

`crates/pdt-api/src/lib.rs` (a stub so the tests compile and then fail):
```rust
mod error;
mod routes;

pub use error::ApiErrorBody;
pub use routes::{AppState, Health, router};
```

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p pdt-api`
Expected: compile errors for the missing `error`/`routes` items.

- [ ] **Step 3: Implement**

`crates/pdt-api/src/error.rs`:
```rust
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pdt_core::project::ValidationError;
use pdt_git::GitError;
use pdt_store::StoreError;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", format!("{what} not found"))
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ApiErrorBody { code: self.code.into(), message: self.message })).into_response()
    }
}

impl From<ValidationError> for ApiError {
    fn from(e: ValidationError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

impl From<GitError> for ApiError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::PathNotFound(_) => Self::new(StatusCode::BAD_REQUEST, "path_not_found", e.to_string()),
            GitError::NotARepo(_) => Self::new(StatusCode::BAD_REQUEST, "not_a_git_repo", e.to_string()),
            GitError::GitMissing => Self::new(StatusCode::INTERNAL_SERVER_ERROR, "git_missing", e.to_string()),
            GitError::Command(_) => Self::internal(e),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(_) => {
                Self::new(StatusCode::CONFLICT, "conflict", "a project for this repository already exists")
            }
            other => Self::internal(other),
        }
    }
}
```

`crates/pdt-api/src/routes.rs`:
```rust
use crate::error::ApiError;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pdt_core::brand::PRODUCT_NAME;
use pdt_core::project::{NewProject, Project};
use pdt_store::Store;
use serde::Serialize;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};
use ts_rs::TS;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Health {
    pub status: String,
    pub name: String,
    pub version: String,
}

pub fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", get(get_project))
        .fallback(|| async { ApiError::not_found("route") })
        .with_state(state);
    let app = Router::new().nest("/api", api);
    match web_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => app,
    }
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok".into(), name: PRODUCT_NAME.into(), version: env!("CARGO_PKG_VERSION").into() })
}

async fn list_projects(State(st): State<AppState>) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(st.store.list_projects().await?))
}

async fn get_project(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Project>, ApiError> {
    st.store.get_project(&id).await?.map(Json).ok_or_else(|| ApiError::not_found("project"))
}

async fn create_project(
    State(st): State<AppState>,
    Json(input): Json<NewProject>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let valid = input.validate()?;
    let path = PathBuf::from(&valid.repo_path);
    let info = tokio::task::spawn_blocking(move || pdt_git::resolve_repo(&path)).await.map_err(ApiError::internal)??;
    let project = Project {
        id: uuid::Uuid::now_v7().to_string(),
        name: valid.name,
        repo_path: info.root.to_string_lossy().into_owned(),
        base_branch: valid.base_branch.or(info.current_branch).unwrap_or_else(|| "main".into()),
        created_at: jiff::Timestamp::now().as_millisecond(),
    };
    st.store.insert_project(&project).await?;
    Ok((StatusCode::CREATED, Json(project)))
}
```

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo test -p pdt-api`
Expected: 5 tests pass. `web/src/api/gen/Health.ts` and `ApiErrorBody.ts` now exist.

If the nested router's fallback isn't used for `/api/nope` in axum 0.8.9 (you get the single-page-app HTML instead of JSON), replace `.nest("/api", api)` with routes registered on the outer router under full `/api/...` paths, plus `.route("/api/{*rest}", get(|| async { ApiError::not_found("route") }))`. Keep the test unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-api web/src/api/gen Cargo.lock
git commit -m "feat(api): health + projects endpoints, JSON errors, SPA fallback"
```

---

### Task 6: The `pdtd` service program

**Files:**
- Create: `crates/pdtd/Cargo.toml`, `crates/pdtd/src/main.rs`, `crates/pdtd/src/config.rs`

**Interfaces:**
- Consumes: `brand::{Profile, DAEMON_BIN}`, `Store::open`, `pdt_api::{router, AppState}`
- Produces:
  - The `pdtd` program.
  - `config::Config { profile, port, data_dir, web_dir }` and `Config::from_lookup(lookup: impl Fn(&str) -> Option<String>, default_data_root: Option<PathBuf>, exe_dir: Option<PathBuf>) -> anyhow::Result<Config>`.
  - `bind(port) -> anyhow::Result<TcpListener>`, which reports a port already in use clearly.
  - Environment variables: `PDT_PROFILE`, `PDT_PORT`, `PDT_DATA_DIR`, `PDT_WEB_DIR`, `RUST_LOG`.

- [ ] **Step 1: Create the crate and write the failing tests**

`crates/pdtd/Cargo.toml`:
```toml
[package]
name = "pdtd"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[dependencies]
pdt-core.workspace = true
pdt-store.workspace = true
pdt-api.workspace = true
tokio.workspace = true
axum.workspace = true
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

Then run: `cargo add dirs -p pdtd`

`crates/pdtd/src/config.rs` (tests first):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn stable_defaults() {
        let c = Config::from_lookup(lookup(&[]), Some(PathBuf::from("/data")), None).unwrap();
        assert_eq!(c.profile, Profile::Stable);
        assert_eq!(c.port, 7420);
        assert_eq!(c.data_dir, PathBuf::from("/data").join("pdt"));
        assert_eq!(c.web_dir, None);
    }

    #[test]
    fn dev_profile_and_overrides() {
        let c = Config::from_lookup(lookup(&[("PDT_PROFILE", "dev")]), Some(PathBuf::from("/data")), None).unwrap();
        assert_eq!((c.port, c.data_dir.clone()), (7421, PathBuf::from("/data").join("pdt-dev")));

        let c = Config::from_lookup(
            lookup(&[("PDT_PORT", "9999"), ("PDT_DATA_DIR", "/x"), ("PDT_WEB_DIR", "/w")]),
            None,
            None,
        )
        .unwrap();
        assert_eq!((c.port, c.data_dir, c.web_dir), (9999, PathBuf::from("/x"), Some(PathBuf::from("/w"))));
    }

    #[test]
    fn bad_port_is_error() {
        assert!(Config::from_lookup(lookup(&[("PDT_PORT", "abc")]), Some("/d".into()), None).is_err());
    }

    #[test]
    fn web_dir_next_to_exe_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("web")).unwrap();
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(c.web_dir, Some(tmp.path().join("web")));
    }

    #[tokio::test]
    async fn port_in_use_is_reported() {
        let held = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = held.local_addr().unwrap().port();
        let err = bind(port).await.unwrap_err().to_string();
        assert!(err.contains("already in use"), "{err}");
    }
}
```

Then run: `cargo add --dev tempfile -p pdtd`

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p pdtd`
Expected: compile errors: `cannot find type Config`.

- [ ] **Step 3: Implement**

`crates/pdtd/src/config.rs`, above the tests:
```rust
use anyhow::{Context, bail};
use pdt_core::brand::{DAEMON_BIN, Profile};
use std::path::PathBuf;
use tokio::net::TcpListener;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub profile: Profile,
    pub port: u16,
    pub data_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
}

impl Config {
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        default_data_root: Option<PathBuf>,
        exe_dir: Option<PathBuf>,
    ) -> anyhow::Result<Config> {
        let profile = Profile::from_env_value(lookup("PDT_PROFILE").as_deref());
        let port = match lookup("PDT_PORT") {
            Some(p) => p.parse::<u16>().with_context(|| format!("PDT_PORT is not a valid port: {p:?}"))?,
            None => profile.port(),
        };
        let data_dir = match lookup("PDT_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => match default_data_root {
                Some(root) => root.join(profile.data_dir_name()),
                None => bail!("could not determine a data directory; set PDT_DATA_DIR"),
            },
        };
        let web_dir = lookup("PDT_WEB_DIR")
            .map(PathBuf::from)
            .or_else(|| exe_dir.map(|d| d.join("web")).filter(|d| d.is_dir()));
        Ok(Config { profile, port, data_dir, web_dir })
    }

    pub fn from_env() -> anyhow::Result<Config> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
        Self::from_lookup(|k| std::env::var(k).ok(), dirs::data_dir(), exe_dir)
    }
}

pub async fn bind(port: u16) -> anyhow::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            bail!("port {port} is already in use — is another {DAEMON_BIN} running? (set PDT_PORT to override)")
        }
        Err(e) => Err(e).with_context(|| format!("failed to bind 127.0.0.1:{port}")),
    }
}
```

`crates/pdtd/src/main.rs`:
```rust
mod config;

use pdt_api::{AppState, router};
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME};
use pdt_store::Store;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    if let Err(e) = run().await {
        eprintln!("{DAEMON_BIN}: {e:#}");
        std::process::exit(2);
    }
}

async fn run() -> anyhow::Result<()> {
    let cfg = config::Config::from_env()?;
    std::fs::create_dir_all(&cfg.data_dir)?;
    let store = Store::open(&cfg.data_dir.join("pdt.db")).await?;
    let listener = config::bind(cfg.port).await?;
    tracing::info!(
        profile = ?cfg.profile, port = cfg.port, data_dir = %cfg.data_dir.display(),
        web_dir = ?cfg.web_dir, "{PRODUCT_NAME} daemon listening on http://127.0.0.1:{}", cfg.port
    );
    axum::serve(listener, router(AppState { store }, cfg.web_dir)).await?;
    Ok(())
}
```

- [ ] **Step 4: Run the tests, then try the service by hand**

Run: `cargo test -p pdtd`
Expected: 5 tests pass.

Run (PowerShell): `$env:PDT_PROFILE='dev'; cargo run -p pdtd`, then in a second terminal run `curl http://127.0.0.1:7421/api/health`.
Expected: `{"status":"ok","name":"Personal Dev Team","version":"0.1.0"}`. Starting a second `pdtd` with the dev profile exits with `port 7421 is already in use — is another pdtd running?`.

- [ ] **Step 5: Commit**

```bash
git add crates/pdtd Cargo.lock
git commit -m "feat(pdtd): daemon binary with profiles, env config, clear port-in-use error"
```

---

### Task 7: Web scaffold, design tokens, theme and core components

**Files:**
- Create: `package.json`, `pnpm-workspace.yaml`, `web/package.json`, `web/index.html`, `web/vite.config.ts`, `web/tsconfig.json`, `web/src/test-setup.ts`, `web/src/index.css`, `web/src/main.tsx`, `web/ui/tokens.css`, `web/ui/theme.ts`, `web/ui/theme.test.ts`, `web/ui/Button.tsx`, `web/ui/StatusDot.tsx`, `web/ui/components.test.tsx`, `web/ui/index.ts`

**Interfaces:**
- Produces:
  - `@ui` import alias (maps to `web/ui`)
  - `Button({ variant?: 'primary' | 'secondary' | 'ghost', ...buttonProps })`
  - `StatusDot({ status: 'running' | 'waiting' | 'failed' | 'idle' })`
  - `ThemePref = 'system' | 'light' | 'dark'`
  - `readThemePref(): ThemePref`, `applyThemePref(p: ThemePref): void`
  - Tailwind utility names: `bg-bg`, `bg-surface`, `bg-surface-2`, `border-border`, `text-fg`, `text-muted`, `bg-accent`, `text-on-accent`, `bg-accent-soft`, `bg-running`, `bg-waiting`, `bg-failed`, `bg-idle`, `rounded-pdt`, `font-mono`

- [ ] **Step 1: Set up the JS workspace and install dependencies**

`pnpm-workspace.yaml`:
```yaml
packages:
  - web
  - desktop
```

`package.json`:
```json
{
  "private": true,
  "packageManager": "pnpm@12.11.2",
  "scripts": {
    "test": "pnpm -r test",
    "typecheck": "pnpm -r typecheck",
    "build": "pnpm --dir web build"
  }
}
```

`web/package.json`:
```json
{
  "name": "@pdt/web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "typecheck": "tsc --noEmit",
    "test": "vitest run"
  }
}
```

Run:
```bash
pnpm --dir web add react@19.3.1 react-dom@19.3.1 @tanstack/react-query@5.104.2 lucide-react @fontsource/inter @fontsource/jetbrains-mono
pnpm --dir web add -D typescript@7.0.2 vite@8.3.4 @vitejs/plugin-react@6.1.2 tailwindcss@4.3.3 @tailwindcss/vite vitest@5.0.4 jsdom @testing-library/react @testing-library/user-event @testing-library/jest-dom @types/react @types/react-dom
```

- [ ] **Step 2: Write the config and token files**

`web/vite.config.ts`:
```ts
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@ui': fileURLToPath(new URL('./ui', import.meta.url)) } },
  // Dev server talks to the *dev* daemon (PDT_PROFILE=dev → port 7421).
  server: { proxy: { '/api': 'http://127.0.0.1:7421' } },
  test: { environment: 'jsdom', setupFiles: ['./src/test-setup.ts'], css: false },
});
```

`web/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "types": ["@testing-library/jest-dom"],
    "paths": { "@ui": ["./ui/index.ts"], "@ui/*": ["./ui/*"] }
  },
  "include": ["src", "ui", "vite.config.ts"]
}
```

`web/src/test-setup.ts`:
```ts
import '@testing-library/jest-dom/vitest';
```

`web/index.html`:
```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Personal Dev Team</title>
  </head>
  <body class="bg-bg text-fg font-sans">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`web/ui/tokens.css`:
```css
:root {
  --pdt-bg: #f7f7f9;
  --pdt-surface: #ffffff;
  --pdt-surface-2: #ebebef;
  --pdt-border: #e2e3e8;
  --pdt-fg: #17181d;
  --pdt-muted: #6b7080;
  --pdt-accent: #2563eb;
  --pdt-on-accent: #ffffff;
  --pdt-accent-soft: #e6eefe;
  --pdt-ring: rgba(37, 99, 235, 0.25);
  --pdt-running: #15803d;
  --pdt-waiting: #b45309;
  --pdt-failed: #dc2626;
  --pdt-idle: #6b7080;
  --pdt-radius: 6px;
  --pdt-font-sans: 'Inter', ui-sans-serif, system-ui, sans-serif;
  --pdt-font-mono: 'JetBrains Mono', ui-monospace, monospace;
  color-scheme: light;
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme='light']) {
    --pdt-bg: #0e0f12;
    --pdt-surface: #17181d;
    --pdt-surface-2: #1f2026;
    --pdt-border: #26282f;
    --pdt-fg: #e7e8ec;
    --pdt-muted: #8a8f9c;
    --pdt-accent: #3561b8;
    --pdt-on-accent: #eef2fb;
    --pdt-accent-soft: #171f33;
    --pdt-ring: rgba(53, 97, 184, 0.35);
    --pdt-running: #34d399;
    --pdt-waiting: #f59e0b;
    --pdt-failed: #f87171;
    --pdt-idle: #8a8f9c;
    color-scheme: dark;
  }
}

:root[data-theme='dark'] {
  --pdt-bg: #0e0f12;
  --pdt-surface: #17181d;
  --pdt-surface-2: #1f2026;
  --pdt-border: #26282f;
  --pdt-fg: #e7e8ec;
  --pdt-muted: #8a8f9c;
  --pdt-accent: #3561b8;
  --pdt-on-accent: #eef2fb;
  --pdt-accent-soft: #171f33;
  --pdt-ring: rgba(53, 97, 184, 0.35);
  --pdt-running: #34d399;
  --pdt-waiting: #f59e0b;
  --pdt-failed: #f87171;
  --pdt-idle: #8a8f9c;
  color-scheme: dark;
}
```

`web/src/index.css`:
```css
@import 'tailwindcss';
@import '@fontsource/inter/400.css';
@import '@fontsource/inter/500.css';
@import '@fontsource/inter/600.css';
@import '@fontsource/jetbrains-mono/400.css';
@import '../ui/tokens.css';

@theme inline {
  --color-bg: var(--pdt-bg);
  --color-surface: var(--pdt-surface);
  --color-surface-2: var(--pdt-surface-2);
  --color-border: var(--pdt-border);
  --color-fg: var(--pdt-fg);
  --color-muted: var(--pdt-muted);
  --color-accent: var(--pdt-accent);
  --color-on-accent: var(--pdt-on-accent);
  --color-accent-soft: var(--pdt-accent-soft);
  --color-ring: var(--pdt-ring);
  --color-running: var(--pdt-running);
  --color-waiting: var(--pdt-waiting);
  --color-failed: var(--pdt-failed);
  --color-idle: var(--pdt-idle);
  --radius-pdt: var(--pdt-radius);
  --font-sans: var(--pdt-font-sans);
  --font-mono: var(--pdt-font-mono);
}

html, body, #root { height: 100%; margin: 0; }
```

- [ ] **Step 3: Write the failing tests**

`web/ui/theme.test.ts`:
```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyThemePref, readThemePref } from './theme';

afterEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  vi.restoreAllMocks();
});

describe('theme', () => {
  it('applies explicit themes via data-theme and persists', () => {
    applyThemePref('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(readThemePref()).toBe('dark');
  });

  it('system removes the attribute', () => {
    applyThemePref('light');
    applyThemePref('system');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
    expect(readThemePref()).toBe('system');
  });

  it('ignores corrupted stored values', () => {
    localStorage.setItem('pdt.theme', 'purple');
    expect(readThemePref()).toBe('system');
  });

  it('survives storage that throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
    expect(readThemePref()).toBe('system');
    expect(() => applyThemePref('dark')).not.toThrow();
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });
});
```

`web/ui/components.test.tsx`:
```tsx
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Button, StatusDot } from '@ui';

describe('Button', () => {
  it('primary uses accent tokens', () => {
    render(<Button variant="primary">Send feedback</Button>);
    const b = screen.getByRole('button', { name: 'Send feedback' });
    expect(b.className).toContain('bg-accent');
    expect(b.className).toContain('text-on-accent');
  });

  it('defaults to secondary and forwards props', () => {
    render(<Button disabled type="submit">Approve</Button>);
    const b = screen.getByRole('button', { name: 'Approve' });
    expect(b).toBeDisabled();
    expect(b).toHaveAttribute('type', 'submit');
    expect(b.className).toContain('border-border');
  });
});

describe('StatusDot', () => {
  it('labels and colors by status', () => {
    render(<StatusDot status="waiting" />);
    const dot = screen.getByRole('img', { name: 'waiting' });
    expect(dot.className).toContain('bg-waiting');
  });
});
```

- [ ] **Step 4: Run the tests and check that they fail**

Run: `pnpm --dir web test`
Expected: FAIL. Cannot resolve `./theme` or `@ui`.

- [ ] **Step 5: Implement**

`web/ui/theme.ts`:
```ts
import { useState } from 'react';

export type ThemePref = 'system' | 'light' | 'dark';
const KEY = 'pdt.theme';

export function readThemePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    if (v === 'light' || v === 'dark') return v;
  } catch {
    // storage unavailable (private window, blocked) — fall back to system
  }
  return 'system';
}

export function applyThemePref(pref: ThemePref): void {
  const root = document.documentElement;
  if (pref === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', pref);
  try {
    localStorage.setItem(KEY, pref);
  } catch {
    // non-persistent is acceptable
  }
}

export function useThemePref(): [ThemePref, (p: ThemePref) => void] {
  const [pref, setPref] = useState<ThemePref>(readThemePref);
  return [pref, (p) => { applyThemePref(p); setPref(p); }];
}
```

`web/ui/Button.tsx`:
```tsx
import type { ButtonHTMLAttributes } from 'react';

type Variant = 'primary' | 'secondary' | 'ghost';

const variants: Record<Variant, string> = {
  primary: 'bg-accent text-on-accent font-semibold hover:brightness-110',
  secondary: 'bg-surface text-fg border border-border hover:bg-surface-2',
  ghost: 'text-muted hover:text-fg hover:bg-surface-2',
};

export function Button({
  variant = 'secondary',
  className = '',
  type = 'button',
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      type={type}
      className={`inline-flex items-center gap-1.5 rounded-pdt px-3 py-1.5 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 disabled:pointer-events-none ${variants[variant]} ${className}`}
      {...rest}
    />
  );
}
```

`web/ui/StatusDot.tsx`:
```tsx
export type Status = 'running' | 'waiting' | 'failed' | 'idle';

const color: Record<Status, string> = {
  running: 'bg-running',
  waiting: 'bg-waiting',
  failed: 'bg-failed',
  idle: 'bg-idle',
};

export function StatusDot({ status }: { status: Status }) {
  return <span role="img" aria-label={status} className={`inline-block size-2 rounded-full ${color[status]}`} />;
}
```

`web/ui/index.ts`:
```ts
export { Button } from './Button';
export { StatusDot, type Status } from './StatusDot';
export { applyThemePref, readThemePref, useThemePref, type ThemePref } from './theme';
```

`web/src/main.tsx` (temporary entry point; Task 8 replaces the body with the app frame):
```tsx
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { applyThemePref, readThemePref, Button } from '@ui';
import './index.css';

applyThemePref(readThemePref());

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <div className="p-6"><Button variant="primary">Personal Dev Team</Button></div>
  </StrictMode>,
);
```

- [ ] **Step 6: Run the tests, type check and build, and check that they pass**

Run: `pnpm --dir web test; pnpm --dir web build`
Expected: 7 tests pass, the type check is clean, and `web/dist/index.html` is created.

If `tsc` from TypeScript 7 rejects an option in `tsconfig.json`, remove only that option and note the reason in the commit message.

- [ ] **Step 7: Commit**

```bash
git add package.json pnpm-workspace.yaml pnpm-lock.yaml web
git commit -m "feat(web): vite/react/tailwind scaffold, Graphite tokens, theme, Button, StatusDot"
```

---

### Task 8: API client and app frame (icon bar, list column, work area, bottom panel, projects)

**Files:**
- Create: `web/src/api/client.ts`, `web/src/api/client.test.ts`, `web/src/shell/views.ts`, `web/src/shell/IconRail.tsx`, `web/src/shell/BottomPanel.tsx`, `web/src/shell/AppShell.tsx`, `web/src/shell/AppShell.test.tsx`, `web/src/projects/ProjectsList.tsx`, `web/src/projects/AddProjectForm.tsx`, `web/src/projects/projects.test.tsx`
- Modify: `web/src/main.tsx`

**Interfaces:**
- Consumes: the generated types `Project`, `NewProject`, `Health`, `ApiErrorBody` from `web/src/api/gen/*`, and `Button` and `StatusDot` from `@ui`.
- Produces:
  - `api.health()`, `api.listProjects()`, `api.createProject(p: NewProject)`
  - `class ApiError extends Error { status: number; code: string }`
  - `AppShell` component
  - `ViewId = 'inbox' | 'board' | 'sessions' | 'personas' | 'files'`
  - Ctrl+\` (backtick) toggles the bottom panel

- [ ] **Step 1: Write the failing tests**

`web/src/api/client.test.ts`:
```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError } from './client';

afterEach(() => vi.unstubAllGlobals());

function stubFetch(status: number, body: unknown) {
  // A fresh Response per call: a Response body can only be read once.
  const fetchMock = vi.fn().mockImplementation(
    async () => new Response(typeof body === 'string' ? body : JSON.stringify(body), { status }),
  );
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

describe('api client', () => {
  it('returns parsed JSON on success', async () => {
    stubFetch(200, [{ id: '1', name: 'p', repoPath: '/r', baseBranch: 'main', createdAt: 1 }]);
    expect((await api.listProjects())[0].name).toBe('p');
  });

  it('throws ApiError with server code and message', async () => {
    stubFetch(409, { code: 'conflict', message: 'a project for this repository already exists' });
    await expect(api.createProject({ name: 'x', repoPath: '/r' })).rejects.toMatchObject({
      status: 409, code: 'conflict', message: 'a project for this repository already exists',
    });
  });

  it('handles non-JSON error bodies', async () => {
    stubFetch(502, 'Bad Gateway');
    const err = await api.health().catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('http_error');
    expect(err.message).toBe('HTTP 502');
  });
});
```

`web/src/shell/AppShell.test.tsx`:
```tsx
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AppShell } from './AppShell';

function renderShell() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}><AppShell /></QueryClientProvider>);
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response('[]', { status: 200 })));
});

describe('AppShell', () => {
  it('renders the five rail views and marks the active one', async () => {
    renderShell();
    for (const name of ['Inbox', 'Board', 'Sessions', 'Personas', 'Files']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Board' }));
    expect(screen.getByRole('button', { name: 'Board' })).toHaveAttribute('aria-current', 'page');
  });

  it('toggles the bottom panel with Ctrl+`', async () => {
    renderShell();
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
  });
});
```

`web/src/projects/projects.test.tsx`:
```tsx
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProjectsList } from './ProjectsList';
import { AddProjectForm } from './AddProjectForm';

afterEach(() => vi.unstubAllGlobals());

function wrap(ui: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

describe('projects', () => {
  it('lists projects from the API', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response(
      JSON.stringify([{ id: '1', name: 'PDT', repoPath: 'C:\\work\\pdt', baseBranch: 'main', createdAt: 1 }]),
      { status: 200 },
    )));
    wrap(<ProjectsList selectedId={null} onSelect={() => {}} />);
    expect(await screen.findByText('PDT')).toBeInTheDocument();
  });

  it('shows the server error message when adding fails', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response(
      JSON.stringify({ code: 'not_a_git_repo', message: 'not a git repository: C:\\tmp' }), { status: 400 },
    )));
    wrap(<AddProjectForm />);
    await userEvent.type(screen.getByLabelText('Name'), 'x');
    await userEvent.type(screen.getByLabelText('Repository path'), 'C:\\tmp');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('not a git repository: C:\\tmp');
  });
});
```

- [ ] **Step 2: Run the tests and check that they fail**

Run: `pnpm --dir web test`
Expected: FAIL. Cannot resolve `./client`, `./AppShell` and `./ProjectsList`.

- [ ] **Step 3: Implement**

`web/src/api/client.ts`:
```ts
import type { ApiErrorBody } from './gen/ApiErrorBody';
import type { Health } from './gen/Health';
import type { NewProject } from './gen/NewProject';
import type { Project } from './gen/Project';

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message);
    this.name = 'ApiError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, { ...init, headers: { 'content-type': 'application/json', ...init?.headers } });
  if (!res.ok) {
    let body: Partial<ApiErrorBody> = {};
    try {
      body = await res.json();
    } catch {
      // non-JSON error body (proxy, crash page)
    }
    throw new ApiError(res.status, body.code ?? 'http_error', body.message ?? `HTTP ${res.status}`);
  }
  return (await res.json()) as T;
}

export const api = {
  health: () => request<Health>('/api/health'),
  listProjects: () => request<Project[]>('/api/projects'),
  createProject: (p: NewProject) => request<Project>('/api/projects', { method: 'POST', body: JSON.stringify(p) }),
};
```

`web/src/shell/views.ts`:
```ts
import { Bot, Files, Inbox, KanbanSquare, TerminalSquare, type LucideIcon } from 'lucide-react';

export type ViewId = 'inbox' | 'board' | 'sessions' | 'personas' | 'files';

export const VIEWS: { id: ViewId; label: string; icon: LucideIcon }[] = [
  { id: 'inbox', label: 'Inbox', icon: Inbox },
  { id: 'board', label: 'Board', icon: KanbanSquare },
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'personas', label: 'Personas', icon: Bot },
  { id: 'files', label: 'Files', icon: Files },
];
```

`web/src/shell/IconRail.tsx`:
```tsx
import { VIEWS, type ViewId } from './views';

export function IconRail({ active, onSelect }: { active: ViewId; onSelect: (v: ViewId) => void }) {
  return (
    <nav aria-label="Views" className="flex md:flex-col gap-1 p-1.5 bg-surface border-border md:border-r border-t md:border-t-0 justify-around md:justify-start">
      {VIEWS.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type="button"
          aria-label={label}
          title={label}
          aria-current={active === id ? 'page' : undefined}
          onClick={() => onSelect(id)}
          className={`p-2 rounded-pdt outline-none focus-visible:ring-2 focus-visible:ring-ring ${active === id ? 'bg-accent-soft text-fg' : 'text-muted hover:text-fg hover:bg-surface-2'}`}
        >
          <Icon size={18} aria-hidden />
        </button>
      ))}
    </nav>
  );
}
```

`web/src/shell/BottomPanel.tsx`:
```tsx
export function BottomPanel() {
  return (
    <section aria-label="Terminal panel" className="h-56 border-t border-border bg-surface flex flex-col">
      <div className="flex gap-3 px-3 py-1.5 border-b border-border text-xs text-muted">Terminals</div>
      <div className="flex-1 p-3 font-mono text-xs text-muted">No terminal sessions yet.</div>
    </section>
  );
}
```
(A `<section>` with an accessible name has the `region` role, which the shell test queries.)

`web/src/projects/ProjectsList.tsx`:
```tsx
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';

export function ProjectsList({ selectedId, onSelect }: { selectedId: string | null; onSelect: (id: string) => void }) {
  const q = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  if (q.isPending) return <p className="px-2 text-xs text-muted">Loading…</p>;
  if (q.isError) return <p role="alert" className="px-2 text-xs text-failed">{q.error.message}</p>;
  if (q.data.length === 0) return <p className="px-2 text-xs text-muted">No projects yet.</p>;
  return (
    <ul className="flex flex-col gap-0.5">
      {q.data.map((p) => (
        <li key={p.id}>
          <button
            type="button"
            onClick={() => onSelect(p.id)}
            className={`w-full text-left px-2 py-1 rounded-pdt text-sm truncate ${selectedId === p.id ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
            title={p.repoPath}
          >
            {p.name}
          </button>
        </li>
      ))}
    </ul>
  );
}
```

`web/src/projects/AddProjectForm.tsx`:
```tsx
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';

const field = 'w-full rounded-pdt border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function AddProjectForm() {
  const qc = useQueryClient();
  const [name, setName] = useState('');
  const [repoPath, setRepoPath] = useState('');
  const m = useMutation({
    mutationFn: api.createProject,
    onSuccess: () => {
      setName('');
      setRepoPath('');
      void qc.invalidateQueries({ queryKey: ['projects'] });
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    m.mutate({ name, repoPath });
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-2 p-2">
      <label className="text-xs text-muted">Name<input className={field} value={name} onChange={(e) => setName(e.target.value)} /></label>
      <label className="text-xs text-muted">Repository path<input className={field} value={repoPath} onChange={(e) => setRepoPath(e.target.value)} /></label>
      {m.isError && <p role="alert" className="text-xs text-failed">{m.error.message}</p>}
      <Button variant="primary" type="submit" disabled={m.isPending}>Add project</Button>
    </form>
  );
}
```

`web/src/shell/AppShell.tsx`:
```tsx
import { useEffect, useState } from 'react';
import { ProjectsList } from '../projects/ProjectsList';
import { AddProjectForm } from '../projects/AddProjectForm';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { VIEWS, type ViewId } from './views';

const PHASE_NOTE: Record<Exclude<ViewId, 'sessions'>, string> = {
  inbox: 'The inbox arrives with the pipeline (Phase 5).',
  board: 'The board arrives with tickets (Phase 2).',
  personas: 'Personas arrive with the pipeline (Phase 5).',
  files: 'The file view arrives with review (Phase 4).',
};

export function AppShell() {
  const [view, setView] = useState<ViewId>('sessions');
  const [bottomOpen, setBottomOpen] = useState(false);
  const [projectId, setProjectId] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === '`') {
        e.preventDefault();
        setBottomOpen((o) => !o);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const label = VIEWS.find((v) => v.id === view)!.label;

  return (
    <div className="h-full flex flex-col-reverse md:flex-row bg-bg text-fg">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              <ProjectsList selectedId={projectId} onSelect={setProjectId} />
              <AddProjectForm />
            </>
          ) : (
            <p className="px-2 text-xs text-muted">{PHASE_NOTE[view]}</p>
          )}
        </div>
      </aside>
      <main className="flex-1 flex flex-col min-w-0 min-h-0">
        <div className="flex-1 overflow-auto p-4 text-sm text-muted">
          {projectId ? `Project ${projectId}` : 'Select a project to get started.'}
        </div>
        {bottomOpen && <BottomPanel />}
      </main>
    </div>
  );
}
```

`web/src/main.tsx`:
```tsx
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { applyThemePref, readThemePref } from '@ui';
import { AppShell } from './shell/AppShell';
import './index.css';

applyThemePref(readThemePref());
const queryClient = new QueryClient();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <AppShell />
    </QueryClientProvider>
  </StrictMode>,
);
```

- [ ] **Step 4: Run the tests and check that they pass**

Run: `pnpm --dir web test; pnpm --dir web typecheck`
Expected: all tests pass (7 earlier plus 7 new), and the type check is clean.

- [ ] **Step 5: Check end to end by hand**

1. Run `pnpm --dir web build`.
2. Start the service: `$env:PDT_PROFILE='dev'; $env:PDT_WEB_DIR="$PWD\web\dist"; cargo run -p pdtd`.
3. Open `http://127.0.0.1:7421`.
4. Add this repo as a project. It should appear in the list.
5. Add it again. The message "a project for this repository already exists" should appear.
6. Press Ctrl+\` to toggle the bottom panel.
7. Narrow the window below 768px. The icon bar should move to the bottom.

- [ ] **Step 6: Commit**

```bash
git add web
git commit -m "feat(web): API client, app shell (rail, list, work area, bottom panel), projects"
```

---

### Task 9: Tauri desktop window that starts or attaches to `pdtd`

**Files:**
- Create: `desktop/package.json`, `desktop/app-icon.svg`, `desktop/src-tauri/Cargo.toml`, `desktop/src-tauri/build.rs`, `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/src/main.rs`, `desktop/src-tauri/src/daemon.rs`
- Modify: `Cargo.toml` (workspace `members`, `default-members`)

**Interfaces:**
- Consumes: `brand::{Profile, DAEMON_BIN, PRODUCT_NAME}`
- Produces:
  - `daemon::is_listening(port: u16) -> bool`
  - `daemon::ensure_daemon(port: u16, spawn: impl FnOnce() -> std::io::Result<()>, timeout: Duration) -> Result<(), String>`
  - `daemon::spawn_pdtd(profile: Profile) -> std::io::Result<()>`
  - The `pdt-desktop` program.

- [ ] **Step 1: Set up the workspace and package**

Edit the root `Cargo.toml` `[workspace]` section:
```toml
members = ["crates/*", "desktop/src-tauri"]
default-members = ["crates/pdt-core", "crates/pdt-git", "crates/pdt-store", "crates/pdt-api", "crates/pdtd"]
```

`default-members` keeps a plain `cargo test` from building Tauri, which needs `web/dist` to exist.

`desktop/package.json`:
```json
{
  "name": "@pdt/desktop",
  "private": true,
  "scripts": {
    "tauri": "tauri",
    "test": "cargo test -p pdt-desktop",
    "typecheck": "echo no-ts"
  }
}
```

Run: `pnpm --dir desktop add -D @tauri-apps/cli@2.12.3`

`desktop/app-icon.svg`:
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <rect width="1024" height="1024" rx="200" fill="#0e0f12"/>
  <g fill="#3561b8">
    <circle cx="312" cy="312" r="72"/><circle cx="512" cy="312" r="72"/><circle cx="712" cy="312" r="72"/>
    <circle cx="312" cy="512" r="72"/><circle cx="512" cy="512" r="72"/>
    <circle cx="312" cy="712" r="72"/>
  </g>
  <circle cx="712" cy="712" r="72" fill="#e7e8ec"/>
</svg>
```

Run: `pnpm --dir desktop tauri icon app-icon.svg --output src-tauri/icons`
Expected: `desktop/src-tauri/icons/` contains `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`.

`desktop/src-tauri/Cargo.toml`:
```toml
[package]
name = "pdt-desktop"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[build-dependencies]
tauri-build = "2.7.3"

[dependencies]
tauri = "2.12.4"
pdt-core.workspace = true
```

`desktop/src-tauri/build.rs`:
```rust
fn main() {
    tauri_build::build()
}
```

`desktop/src-tauri/tauri.conf.json`:
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Personal Dev Team",
  "version": "0.1.0",
  "identifier": "dev.personaldevteam.desktop",
  "build": { "frontendDist": "../../web/dist" },
  "app": { "windows": [], "security": { "csp": null } },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"]
  }
}
```

- [ ] **Step 2: Write the failing tests**

`desktop/src-tauri/src/daemon.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::net::TcpListener;

    #[test]
    fn does_not_spawn_when_already_listening() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let spawned = Cell::new(false);
        ensure_daemon(port, || { spawned.set(true); Ok(()) }, Duration::from_millis(500)).unwrap();
        assert!(!spawned.get());
    }

    #[test]
    fn spawns_and_waits_until_listening() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let spawn = || {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                let l = TcpListener::bind(("127.0.0.1", port)).unwrap();
                std::thread::sleep(Duration::from_secs(2));
                drop(l);
            });
            Ok(())
        };
        ensure_daemon(port, spawn, Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn times_out_with_message() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = ensure_daemon(port, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not start"), "{err}");
    }

    #[test]
    fn spawn_failure_is_reported() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = ensure_daemon(port, || Err(std::io::Error::other("nope")), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("failed to start pdtd"), "{err}");
    }
}
```

`desktop/src-tauri/src/main.rs` (minimal for now, so the test module compiles):
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod daemon;

fn main() {}
```

- [ ] **Step 3: Run the tests and check that they fail**

Run: `pnpm --dir web build; cargo test -p pdt-desktop`
Expected: compile error: `cannot find function ensure_daemon`.

- [ ] **Step 4: Implement**

`desktop/src-tauri/src/daemon.rs`, above the tests:
```rust
use pdt_core::brand::{DAEMON_BIN, Profile};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn is_listening(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(200)).is_ok()
}

/// Attaches to a running daemon, or spawns one and waits for it to listen.
pub fn ensure_daemon(
    port: u16,
    spawn: impl FnOnce() -> std::io::Result<()>,
    timeout: Duration,
) -> Result<(), String> {
    if is_listening(port) {
        return Ok(());
    }
    spawn().map_err(|e| format!("failed to start {DAEMON_BIN}: {e}"))?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_listening(port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{DAEMON_BIN} did not start listening on port {port} within {timeout:?}"))
}

fn pdtd_path() -> PathBuf {
    let file = format!("{DAEMON_BIN}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&file)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(file))
}

/// Starts pdtd detached from this window; it keeps running after the window closes.
pub fn spawn_pdtd(profile: Profile) -> std::io::Result<()> {
    let mut cmd = Command::new(pdtd_path());
    cmd.env("PDT_PROFILE", if profile == Profile::Dev { "dev" } else { "stable" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    cmd.spawn().map(|_| ())
}
```

`desktop/src-tauri/src/main.rs`:
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::time::Duration;

fn main() {
    let profile = Profile::from_env_value(std::env::var("PDT_PROFILE").ok().as_deref());
    let port = profile.port();
    tauri::Builder::default()
        .setup(move |app| {
            daemon::ensure_daemon(port, || daemon::spawn_pdtd(profile), Duration::from_secs(10))?;
            let url = format!("http://127.0.0.1:{port}/").parse()?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title(PRODUCT_NAME)
                .inner_size(1440.0, 900.0)
                .min_inner_size(360.0, 480.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the desktop shell");
}
```

- [ ] **Step 5: Run the tests and check that they pass**

Run: `cargo test -p pdt-desktop`
Expected: 4 tests pass.

- [ ] **Step 6: Check by hand**

1. Run `cargo build -p pdtd`.
2. Run `$env:PDT_PROFILE='dev'; $env:PDT_WEB_DIR="$PWD\web\dist"; cargo run -p pdt-desktop`.
3. A window titled "Personal Dev Team" opens showing the app frame, and `pdtd` is running.
4. Close the window. `curl http://127.0.0.1:7421/api/health` still responds.
5. Open the desktop app again. It attaches to the same `pdtd` without starting a second one: check that there is one `pdtd` process with `Get-Process pdtd` (Windows) or `pgrep pdtd` (macOS).

The child process inherits the environment, so `PDT_WEB_DIR` is passed through to the service.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock desktop pnpm-lock.yaml
git commit -m "feat(desktop): tauri v2 shell that attaches to or spawns pdtd"
```

---

### Task 10: CI on Windows and macOS

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: every command from Tasks 1–9.
- Produces: required checks for every push and pull request.

- [ ] **Step 1: Write the workflow**

`.github/workflows/ci.yml`:
```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        os: [windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - name: Rust toolchain (from rust-toolchain.toml)
        run: rustup show active-toolchain || rustup toolchain install
      - uses: Swatinem/rust-cache@v2
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 24
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
      - name: Generated TS types are committed
        shell: bash
        run: git diff --exit-code -- web/src/api/gen
      - run: pnpm --dir web typecheck
      - run: pnpm --dir web test
      - run: pnpm --dir web build
      - run: cargo test -p pdt-desktop
```

Before committing, check the GitHub Actions marketplace for the latest major version of each action (`actions/checkout`, `Swatinem/rust-cache`, `pnpm/action-setup`, `actions/setup-node`) and use those tags.

- [ ] **Step 2: Run the same checks locally**

Run:
1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. `git status --porcelain web/src/api/gen`
5. `pnpm --dir web test`
6. `pnpm --dir web build`
7. `cargo test -p pdt-desktop`

Expected: every command succeeds, and the `git status` command prints nothing (generated types are committed). Fix any formatting or clippy findings in the files that triggered them.

- [ ] **Step 3: Commit, push and check CI**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: windows + macos build, lint, test, TS binding drift check"
git push
gh run watch --exit-status
```

Expected: both the Windows and macOS jobs go green.

---

## Phase 0 Done When

- `cargo test`, `pnpm --dir web test` and `cargo test -p pdt-desktop` pass on Windows and macOS in CI.
- The desktop app opens the Graphite app frame, registers projects, and starts or attaches to a `pdtd` that keeps running after the window closes.
- The dev profile (`PDT_PROFILE=dev`, port 7421, data folder `pdt-dev`) runs alongside a stable install without interfering.

**Next plan:** Phase 1, Terminals (PTY sessions, xterm, worktrees). After Phase 1, PDT development moves into PDT itself.
