//! Scryer's whole GraphQL API for the PM and workflows (PDT-150): the actions `scryer_schema` (a tool only) finds and describes
//! operations and types, query (`scryer_query`, `scryer.query`) runs a query document and mutate (`scryer_mutate`, an
//! asking tool, and `scryer.mutate`) runs one mutation. Every document is checked against Scryer's schema
//! before it is sent: the live one from introspection, kept per connection for a while, else the one PDT bundles
//! (`schema.graphql`). Credentials are redacted from what comes back, and a mutation can take a connection's secret by
//! reference (`{"$secret": "<connection>"}`), filled in here so the agent never sees it. Subscriptions are not
//! offered.

use crate::actions::{CONNECTION, Client, THEN};
use crate::core::{MutationRule, REDACTED, SCRYER, mutation_rule, redact, secret_ref};
use apollo_compiler::ast::OperationType;
use apollo_compiler::response::JsonMap;
use apollo_compiler::schema::{ExtendedType, FieldDefinition, InputValueDefinition};
use apollo_compiler::validation::{DiagnosticList, Valid};
use apollo_compiler::{ExecutableDocument, Node, Schema};
use pdt_api::sdk::AppState;
use pdt_api::sdk::BoxFut;
use pdt_api::sdk::{Action, ActionCx, ConnectionFace, NodeFace, ToolFace, Via};
use pdt_core::integration::ToolDecl;
use pdt_core::pm::AskTarget;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ts_rs::TS;

/// Scryer's schema as PDT bundles it, for a Scryer that doesn't answer introspection.
const BUNDLED: &str = include_str!("schema.graphql");
/// How long a Scryer's live schema is kept.
const SCHEMA_TTL: Duration = Duration::from_secs(10 * 60);
/// How long a Scryer that didn't answer introspection uses the bundled schema before PDT asks again.
const FALLBACK_TTL: Duration = Duration::from_secs(60);
/// The most a tool's answer holds, in bytes of JSON; longer lists are cut.
const MAX_ANSWER: usize = 48 * 1024;
/// The most an introspection answer may be, in bytes (Scryer's is a few MB of JSON).
const INTROSPECTION_CAP: usize = 32 * 1024 * 1024;
/// The most `scryer_schema` lists for a search.
const SEARCH_LIMIT: usize = 30;
/// Descriptions are cut to this many characters.
const DESCRIPTION_CHARS: usize = 300;

/// A Scryer's schema and where it came from.
pub(crate) struct ScryerSchema {
    pub schema: Valid<Schema>,
    /// From the Scryer itself (introspection), not the bundled copy.
    pub live: bool,
}

impl ScryerSchema {
    fn source(&self) -> &'static str {
        if self.live { "live" } else { "bundled" }
    }
}

/// The bundled schema.
pub(crate) fn bundled() -> Arc<ScryerSchema> {
    static B: OnceLock<Arc<ScryerSchema>> = OnceLock::new();
    B.get_or_init(|| {
        let schema = Schema::parse_and_validate(BUNDLED, "scryer-schema.graphql").expect("the bundled Scryer schema");
        Arc::new(ScryerSchema { schema, live: false })
    })
    .clone()
}

/// Each Scryer's schema by its address, and when it was read.
type Cached = (Instant, Arc<ScryerSchema>);
static SCHEMAS: LazyLock<Mutex<HashMap<String, Cached>>> = LazyLock::new(Default::default);

/// The standard introspection query, without directives (PDT writes none of Scryer's).
const INTROSPECTION: &str = "query PdtIntrospection { __schema { queryType { name } mutationType { name } \
    subscriptionType { name } types { kind name description fields(includeDeprecated: true) { name description \
    args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason } inputFields { ...InputValue } \
    interfaces { ...TypeRef } enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason } \
    possibleTypes { ...TypeRef } } } } \
    fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue } \
    fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name \
    ofType { kind name ofType { kind name ofType { kind name } } } } } } } }";

impl Client {
    /// Scryer's schema: its own (introspection), kept for a while; the bundled one when it doesn't answer.
    pub(crate) async fn schema(&self) -> Arc<ScryerSchema> {
        let key = format!("{}\n{}", self.conn.name, self.conn.url);
        if let Some((at, s)) = SCHEMAS.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            let ttl = if s.live { SCHEMA_TTL } else { FALLBACK_TTL };
            if at.elapsed() < ttl {
                return s.clone();
            }
        }
        let s = match self
            .gql_capped(INTROSPECTION, json!({}), None, INTROSPECTION_CAP)
            .await
            .and_then(|d| live_schema(&d))
        {
            Ok(schema) => Arc::new(ScryerSchema { schema, live: true }),
            Err(e) => {
                tracing::info!(connection = %self.conn.name, error = %e, "Scryer's own schema: using the bundled one");
                bundled()
            }
        };
        SCHEMAS.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (Instant::now(), s.clone()));
        s
    }
}

// ---- introspection → SDL ----

/// A schema from an introspection answer (`data`).
pub(crate) fn live_schema(data: &Value) -> Result<Valid<Schema>, String> {
    let sdl = sdl_of(&data["__schema"])?;
    Schema::parse_and_validate(sdl, "scryer-introspection.graphql")
        .map_err(|e| format!("Scryer's schema doesn't validate: {}", messages(&e.errors).join("; ")))
}

fn type_ref(t: &Value) -> Result<String, String> {
    match t["kind"].as_str() {
        Some("NON_NULL") => Ok(format!("{}!", type_ref(&t["ofType"])?)),
        Some("LIST") => Ok(format!("[{}]", type_ref(&t["ofType"])?)),
        _ => t["name"].as_str().map(String::from).ok_or_else(|| "a type without a name".into()),
    }
}

fn describe(out: &mut String, v: &Value, indent: &str) {
    if let Some(d) = v["description"].as_str().filter(|d| !d.is_empty()) {
        out.push_str(&format!("{indent}{}\n", serde_json::to_string(d).unwrap_or_default()));
    }
}

fn deprecated(v: &Value) -> String {
    if v["isDeprecated"].as_bool() != Some(true) {
        return String::new();
    }
    match v["deprecationReason"].as_str() {
        Some(r) => format!(" @deprecated(reason: {})", serde_json::to_string(r).unwrap_or_default()),
        None => " @deprecated".into(),
    }
}

fn input_value(v: &Value) -> Result<String, String> {
    let mut s = format!("{}: {}", v["name"].as_str().unwrap_or_default(), type_ref(&v["type"])?);
    if let Some(d) = v["defaultValue"].as_str() {
        s.push_str(&format!(" = {d}"));
    }
    Ok(s)
}

fn sdl_of(schema: &Value) -> Result<String, String> {
    let types = schema["types"].as_array().ok_or("the answer has no types")?;
    let mut out = String::new();
    for t in types {
        let name = t["name"].as_str().unwrap_or_default();
        if name.starts_with("__") {
            continue;
        }
        let names =
            |k: &str| -> Result<Vec<String>, String> { t[k].as_array().into_iter().flatten().map(type_ref).collect() };
        let fields = |k: &str, args: bool| -> Result<String, String> {
            let mut s = String::new();
            for f in t[k].as_array().into_iter().flatten() {
                describe(&mut s, f, "  ");
                if args {
                    let a: Vec<String> =
                        f["args"].as_array().into_iter().flatten().map(input_value).collect::<Result<_, _>>()?;
                    let a = if a.is_empty() { String::new() } else { format!("({})", a.join(", ")) };
                    s.push_str(&format!(
                        "  {}{a}: {}{}\n",
                        f["name"].as_str().unwrap_or_default(),
                        type_ref(&f["type"])?,
                        deprecated(f)
                    ));
                } else {
                    s.push_str(&format!("  {}\n", input_value(f)?));
                }
            }
            Ok(s)
        };
        let implements = |list: Vec<String>| {
            if list.is_empty() { String::new() } else { format!(" implements {}", list.join(" & ")) }
        };
        match t["kind"].as_str() {
            Some("SCALAR") if ["String", "Int", "Float", "Boolean", "ID"].contains(&name) => continue,
            Some("SCALAR") => {
                describe(&mut out, t, "");
                out.push_str(&format!("scalar {name}\n\n"));
            }
            Some("OBJECT") => {
                describe(&mut out, t, "");
                out.push_str(&format!(
                    "type {name}{} {{\n{}}}\n\n",
                    implements(names("interfaces")?),
                    fields("fields", true)?
                ));
            }
            Some("INTERFACE") => {
                describe(&mut out, t, "");
                out.push_str(&format!(
                    "interface {name}{} {{\n{}}}\n\n",
                    implements(names("interfaces")?),
                    fields("fields", true)?
                ));
            }
            Some("UNION") => {
                describe(&mut out, t, "");
                out.push_str(&format!("union {name} = {}\n\n", names("possibleTypes")?.join(" | ")));
            }
            Some("ENUM") => {
                describe(&mut out, t, "");
                let mut s = String::new();
                for v in t["enumValues"].as_array().into_iter().flatten() {
                    describe(&mut s, v, "  ");
                    s.push_str(&format!("  {}{}\n", v["name"].as_str().unwrap_or_default(), deprecated(v)));
                }
                out.push_str(&format!("enum {name} {{\n{s}}}\n\n"));
            }
            Some("INPUT_OBJECT") => {
                describe(&mut out, t, "");
                out.push_str(&format!("input {name} {{\n{}}}\n\n", fields("inputFields", false)?));
            }
            other => return Err(format!("a type of kind {other:?}")),
        }
    }
    let root = |k: &str| schema[k]["name"].as_str().map(String::from);
    let mut roots = vec![format!("  query: {}", root("queryType").ok_or("the answer has no query type")?)];
    if let Some(m) = root("mutationType") {
        roots.push(format!("  mutation: {m}"));
    }
    if let Some(s) = root("subscriptionType") {
        roots.push(format!("  subscription: {s}"));
    }
    out.push_str(&format!("schema {{\n{}\n}}\n", roots.join("\n")));
    Ok(out)
}

// ---- checking documents ----

/// Each error's message and line.
fn messages(errors: &DiagnosticList) -> Vec<String> {
    errors
        .iter()
        .map(|d| {
            let e = d.to_json();
            match e.locations.first() {
                Some(l) => format!("{} (line {})", e.message, l.line),
                None => e.message,
            }
        })
        .collect()
}

/// `document` checked against `schema`; only queries when `queries_only`. The operation to run (by
/// `operation_name`, or the only one), with its variables checked too.
fn check_document(
    schema: &Valid<Schema>,
    document: &str,
    operation_name: Option<&str>,
    variables: &Value,
    queries_only: bool,
) -> Result<Valid<ExecutableDocument>, String> {
    let doc = ExecutableDocument::parse_and_validate(schema, document, "document.graphql").map_err(|e| {
        format!(
            "the document doesn't fit Scryer's schema: {} (look names up with scryer_schema)",
            messages(&e.errors).join("; ")
        )
    })?;
    if queries_only && let Some(op) = doc.operations.iter().find(|o| o.operation_type != OperationType::Query) {
        return Err(format!(
            "scryer_query runs queries only; this document has a {}: changes go through scryer_mutate",
            op.operation_type
        ));
    }
    let op = doc.operations.get(operation_name).map_err(|e| format!("{} (name it with operationName)", e.message()))?;
    let vars = if variables.is_null() { json!({}) } else { variables.clone() };
    let map: JsonMap = serde_json::from_value(vars).map_err(|_| "variables must be an object".to_string())?;
    apollo_compiler::request::coerce_variable_values(schema, op, &map)
        .map_err(|e| format!("the variables don't fit: {}", e.message()))?;
    Ok(doc)
}

/// `v` with every `$secret` reference replaced by `with(connection)`.
fn replace_refs(v: &Value, with: &dyn Fn(&str) -> Value) -> Value {
    if let Some(c) = secret_ref(v) {
        return with(c);
    }
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), replace_refs(x, with))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|x| replace_refs(x, with)).collect()),
        _ => v.clone(),
    }
}

fn refs(v: &Value, found: &mut Vec<String>) {
    if let Some(c) = secret_ref(v) {
        found.push(c.to_string());
        return;
    }
    match v {
        Value::Object(m) => m.values().for_each(|x| refs(x, found)),
        Value::Array(a) => a.iter().for_each(|x| refs(x, found)),
        _ => {}
    }
}

/// `v` with its `$secret` references filled in from PDT's connections, and the secrets put in.
async fn resolve_secrets(st: &AppState, v: &Value) -> Result<(Value, Vec<String>), String> {
    let mut names = vec![];
    refs(v, &mut names);
    names.sort();
    names.dedup();
    let mut secrets: HashMap<String, String> = HashMap::new();
    for n in names {
        let (_, secret) = pdt_api::sdk::connections::connection_with_secret(st, &n).await?;
        let s = secret.filter(|s| !s.is_empty()).ok_or_else(|| format!("the connection {n} has no secret"))?;
        secrets.insert(n, s);
    }
    let filled = replace_refs(v, &|c| json!(secrets.get(c).cloned().unwrap_or_default()));
    Ok((filled, secrets.into_values().collect()))
}

/// What Scryer answered, its credentials redacted, also any of `sent` it echoes back.
fn scrub(v: &Value, sent: &[String]) -> Value {
    let r = redact(v);
    if sent.is_empty() {
        return r;
    }
    let mut text = r.to_string();
    for s in sent.iter().filter(|s| s.len() >= 4) {
        let quoted = serde_json::to_string(s).unwrap_or_default();
        text = text.replace(quoted.trim_matches('"'), REDACTED);
    }
    serde_json::from_str(&text).unwrap_or(r)
}

// ---- configs ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScryerQueryConfig {
    /// A `scryer` connection.
    #[schemars(extend("x-connection-kind" = "scryer"))]
    pub connection: String,
    /// A GraphQL query document (`query { indexers { id name isEnabled } }`): checked against Scryer's schema
    /// first. No mutations or subscriptions.
    pub query: String,
    /// Its variables.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null")]
    pub variables: Option<Value>,
    /// The operation to run, when the document has several.
    #[serde(default)]
    pub operation_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerQueryOutput {
    /// What Scryer answered, credentials redacted.
    #[ts(type = "unknown")]
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScryerMutateConfig {
    /// A `scryer` connection.
    #[schemars(extend("x-connection-kind" = "scryer"))]
    pub connection: String,
    /// The mutation's name (`createIndexerConfig`, `updateGeneralSettings`; scryer_schema lists them).
    pub mutation: String,
    /// Its arguments by name (`{"input": {...}}`). A credential goes in as `{"$secret": "<connection>"}`: PDT puts
    /// that connection's secret there.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null")]
    pub args: Option<Value>,
    /// The fields to read back (`id name isEnabled`); its scalar fields when left out.
    #[serde(default)]
    pub select: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerMutateOutput {
    pub mutation: String,
    /// What the mutation returned, credentials redacted.
    #[ts(type = "unknown")]
    pub result: Value,
}

/// `scryer_schema`'s arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScryerSchemaArgs {
    /// A `scryer` connection.
    pub connection: String,
    /// Words in operation and type names and descriptions (`indexer`, `download client`, `seeding`).
    #[serde(default)]
    pub search: Option<String>,
    /// One query or mutation by name: its arguments, the input types they take, and what it returns.
    #[serde(default)]
    pub operation: Option<String>,
    /// One type by name: its fields, input fields or values.
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
}

// ---- describing the schema ----

fn short(d: Option<&str>) -> Option<String> {
    let d = d?.trim();
    if d.is_empty() {
        return None;
    }
    let cut: String = d.chars().take(DESCRIPTION_CHARS).collect();
    Some(if cut.len() < d.len() { format!("{cut}…") } else { cut })
}

fn root_fields(schema: &Schema, op: OperationType) -> Vec<(&FieldDefinition, &'static str)> {
    let kind = match op {
        OperationType::Query => "query",
        OperationType::Mutation => "mutation",
        OperationType::Subscription => "subscription",
    };
    let Some(root) = schema.root_operation(op) else { return vec![] };
    match schema.types.get(root) {
        Some(ExtendedType::Object(o)) => o.fields.values().map(|f| (&***f, kind)).collect(),
        _ => vec![],
    }
}

fn arg_json(a: &Node<InputValueDefinition>) -> Value {
    let mut v = json!({"name": a.name.as_str(), "type": a.ty.to_string()});
    if let Some(d) = short(a.description.as_deref()) {
        v["description"] = json!(d);
    }
    if let Some(d) = &a.default_value {
        v["default"] = json!(d.to_string());
    }
    v
}

fn signature(f: &FieldDefinition) -> String {
    let args: Vec<String> = f.arguments.iter().map(|a| format!("{}: {}", a.name, a.ty)).collect();
    if args.is_empty() { format!(": {}", f.ty) } else { format!("({}): {}", args.join(", "), f.ty) }
}

fn type_kind(t: &ExtendedType) -> &'static str {
    match t {
        ExtendedType::Scalar(_) => "scalar",
        ExtendedType::Object(_) => "object",
        ExtendedType::Interface(_) => "interface",
        ExtendedType::Union(_) => "union",
        ExtendedType::Enum(_) => "enum",
        ExtendedType::InputObject(_) => "input",
    }
}

/// One type: its fields (with their arguments), input fields, values or members.
pub(crate) fn describe_type(schema: &Schema, name: &str) -> Option<Value> {
    let t = schema.types.get(name)?;
    let mut v = json!({"name": name, "kind": type_kind(t)});
    if let Some(d) = short(t.description().map(|d| d.as_str())) {
        v["description"] = json!(d);
    }
    let field = |f: &FieldDefinition| {
        let mut x = json!({"name": f.name.as_str(), "type": f.ty.to_string()});
        if !f.arguments.is_empty() {
            x["args"] = Value::Array(f.arguments.iter().map(arg_json).collect());
        }
        if let Some(d) = short(f.description.as_deref()) {
            x["description"] = json!(d);
        }
        x
    };
    match t {
        ExtendedType::Object(o) => v["fields"] = o.fields.values().map(|f| field(f)).collect(),
        ExtendedType::Interface(i) => v["fields"] = i.fields.values().map(|f| field(f)).collect(),
        ExtendedType::InputObject(i) => v["inputFields"] = i.fields.values().map(|f| arg_json(f)).collect(),
        ExtendedType::Enum(e) => {
            v["values"] = e
                .values
                .values()
                .map(|x| match short(x.description.as_deref()) {
                    Some(d) => json!({"name": x.value.as_str(), "description": d}),
                    None => json!({"name": x.value.as_str()}),
                })
                .collect()
        }
        ExtendedType::Union(u) => v["members"] = u.members.iter().map(|m| json!(m.name.as_str())).collect(),
        ExtendedType::Scalar(_) => {}
    }
    Some(v)
}

/// The input types `ty` takes, and those inside them (at most `max`).
fn input_types(schema: &Schema, ty: &str, seen: &mut Vec<String>, max: usize) {
    if seen.len() >= max || seen.iter().any(|s| s == ty) {
        return;
    }
    match schema.types.get(ty) {
        Some(ExtendedType::InputObject(i)) => {
            seen.push(ty.to_string());
            for f in i.fields.values() {
                input_types(schema, f.ty.inner_named_type(), seen, max);
            }
        }
        Some(ExtendedType::Enum(_)) => seen.push(ty.to_string()),
        _ => {}
    }
}

/// A query or mutation: its arguments, the input types and enums they take, and what it returns.
pub(crate) fn describe_operation(schema: &Schema, name: &str) -> Option<Value> {
    let (f, kind) = [OperationType::Query, OperationType::Mutation]
        .into_iter()
        .flat_map(|op| root_fields(schema, op))
        .find(|(f, _)| f.name == name)?;
    let mut v = json!({"kind": kind, "name": name, "args": f.arguments.iter().map(arg_json).collect::<Vec<_>>(),
                       "returns": f.ty.to_string()});
    if let Some(d) = short(f.description.as_deref()) {
        v["description"] = json!(d);
    }
    if kind == "mutation" {
        v["asks"] = json!(match mutation_rule(name) {
            MutationRule::Asks => "asks the user unless the connection lets you act",
            MutationRule::AlwaysAsks => "always asks the user",
            MutationRule::Denied => "PDT never sends it: the user does it in Scryer",
        });
    }
    let mut seen = vec![];
    for a in &f.arguments {
        input_types(schema, a.ty.inner_named_type(), &mut seen, 20);
    }
    v["inputTypes"] = seen.iter().filter_map(|t| describe_type(schema, t)).collect();
    if let Some(r) = describe_type(schema, f.ty.inner_named_type()) {
        v["returnType"] = r;
    }
    Some(v)
}

/// Operations and types whose name or description has every word (a name match ranks first).
pub(crate) fn search_schema(schema: &Schema, words: &str) -> Value {
    let words: Vec<String> = words.split_whitespace().map(|w| w.to_lowercase()).collect();
    // A plural matches its singular (`indexers`).
    let has =
        |text: &str, w: &str| text.contains(w) || (w.len() > 3 && w.ends_with('s') && text.contains(&w[..w.len() - 1]));
    let score = |name: &str, description: Option<&str>| -> Option<usize> {
        let (n, d) = (name.to_lowercase(), description.unwrap_or_default().to_lowercase());
        words.iter().all(|w| has(&n, w) || has(&d, w)).then(|| words.iter().filter(|w| has(&n, w)).count())
    };
    let mut found: Vec<(usize, Value)> = vec![];
    for op in [OperationType::Query, OperationType::Mutation] {
        for (f, kind) in root_fields(schema, op) {
            if let Some(s) = score(&f.name, f.description.as_deref()) {
                let mut v = json!({"kind": kind, "name": f.name.as_str(), "signature": signature(f)});
                if let Some(d) = short(f.description.as_deref()) {
                    v["description"] = json!(d);
                }
                found.push((s * 2 + 1, v));
            }
        }
    }
    for (name, t) in &schema.types {
        if t.is_built_in() || name.starts_with("__") {
            continue;
        }
        if let Some(s) = score(name, t.description().map(|d| d.as_str())) {
            let mut v = json!({"kind": type_kind(t), "name": name.as_str()});
            if let Some(d) = short(t.description().map(|d| d.as_str())) {
                v["description"] = json!(d);
            }
            found.push((s * 2, v));
        }
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    let total = found.len();
    let matches: Vec<Value> = found.into_iter().take(SEARCH_LIMIT).map(|(_, v)| v).collect();
    json!({"matches": matches, "more": total.saturating_sub(SEARCH_LIMIT)})
}

/// The answer to `scryer_schema`.
pub(crate) fn schema_answer(s: &ScryerSchema, a: &ScryerSchemaArgs) -> Result<Value, String> {
    let given = |v: &Option<String>| v.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    let schema = &s.schema;
    let mut v = if let Some(op) = given(&a.operation) {
        describe_operation(schema, &op)
            .ok_or_else(|| format!("Scryer has no query or mutation {op}: find it with scryer_schema's search"))?
    } else if let Some(t) = given(&a.type_name) {
        describe_type(schema, &t)
            .ok_or_else(|| format!("Scryer has no type {t}: find it with scryer_schema's search"))?
    } else if let Some(w) = given(&a.search) {
        search_schema(schema, &w)
    } else {
        let count = |op| root_fields(schema, op).len();
        json!({"queries": count(OperationType::Query), "mutations": count(OperationType::Mutation),
               "subscriptions": count(OperationType::Subscription),
               "hint": "search words (`indexer`, `download client`, `settings`), then describe an operation or a type; \
                        read with scryer_query, change with scryer_mutate. Subscriptions are not offered."})
    };
    v["schema"] = json!(s.source());
    Ok(v)
}

// ---- running ----

/// The selection for a value of type `ty`: its scalar fields, and those of its object fields to `depth` (fields that
/// need arguments left out).
fn default_selection(schema: &Schema, ty: &str, depth: usize) -> String {
    let fields = match schema.types.get(ty) {
        Some(ExtendedType::Object(o)) => &o.fields,
        Some(ExtendedType::Interface(i)) => &i.fields,
        Some(ExtendedType::Union(_)) => return "{ __typename }".into(),
        _ => return String::new(),
    };
    let mut parts = vec![];
    for f in fields.values() {
        if f.arguments.iter().any(|a| a.ty.is_non_null() && a.default_value.is_none()) {
            continue;
        }
        match schema.types.get(f.ty.inner_named_type()) {
            Some(ExtendedType::Scalar(_) | ExtendedType::Enum(_)) => parts.push(f.name.to_string()),
            Some(ExtendedType::Object(_) | ExtendedType::Interface(_)) if depth > 1 => {
                let sub = default_selection(schema, f.ty.inner_named_type(), depth - 1);
                if !sub.is_empty() && sub != "{ __typename }" {
                    parts.push(format!("{} {sub}", f.name));
                }
            }
            _ => {}
        }
        if parts.len() >= 60 {
            break;
        }
    }
    if parts.is_empty() { "{ __typename }".into() } else { format!("{{ {} }}", parts.join(" ")) }
}

/// The document a `scryer_mutate` call sends: the one mutation, its arguments as variables typed as the schema says.
pub(crate) fn mutation_document(
    schema: &Schema,
    mutation: &str,
    args: &serde_json::Map<String, Value>,
    select: Option<&str>,
) -> Result<String, String> {
    let m = mutation.trim();
    let Some((f, _)) = root_fields(schema, OperationType::Mutation).into_iter().find(|(f, _)| f.name == m) else {
        let like: Vec<String> = root_fields(schema, OperationType::Mutation)
            .into_iter()
            .map(|(f, _)| f.name.to_string())
            .filter(|n| n.to_lowercase().contains(&m.to_lowercase()) || m.to_lowercase().contains(&n.to_lowercase()))
            .take(8)
            .collect();
        let hint = if like.is_empty() { String::new() } else { format!(" (like: {})", like.join(", ")) };
        return Err(format!("Scryer has no mutation {m}{hint}: find it with scryer_schema"));
    };
    let mut vars = vec![];
    let mut pass = vec![];
    for k in args.keys() {
        let Some(a) = f.arguments.iter().find(|a| a.name == k.as_str()) else {
            return Err(format!("{m} takes no argument {k}; it takes {}", signature(f)));
        };
        vars.push(format!("${k}: {}", a.ty));
        pass.push(format!("{k}: ${k}"));
    }
    let returns = f.ty.inner_named_type();
    let composite = matches!(
        schema.types.get(returns),
        Some(ExtendedType::Object(_) | ExtendedType::Interface(_) | ExtendedType::Union(_))
    );
    let selection = match select.map(str::trim).filter(|s| !s.is_empty()) {
        Some(_) if !composite => return Err(format!("{m} returns {}: leave select out", f.ty)),
        Some(s) if s.starts_with('{') => s.to_string(),
        Some(s) => format!("{{ {s} }}"),
        None if composite => default_selection(schema, returns, 2),
        None => String::new(),
    };
    let vars = if vars.is_empty() { String::new() } else { format!("({})", vars.join(", ")) };
    let pass = if pass.is_empty() { String::new() } else { format!("({})", pass.join(", ")) };
    Ok(format!("mutation PdtMutate{vars} {{ {m}{pass} {selection} }}"))
}

/// Runs a query document, checked first; Scryer's `data`, credentials redacted.
pub(crate) async fn query(client: &Client, c: &ScryerQueryConfig) -> Result<Value, String> {
    let variables = c.variables.clone().unwrap_or(Value::Null);
    if crate::core::has_secret_ref(&variables) {
        return Err("a query sends no secrets: $secret references go in scryer_mutate".into());
    }
    let s = client.schema().await;
    let name = c.operation_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    check_document(&s.schema, &c.query, name, &variables, true)?;
    let vars = if variables.is_null() { json!({}) } else { variables };
    let data = client.gql_named(&c.query, vars, name).await?;
    Ok(redact(&data))
}

/// Runs one mutation, checked first, with its `$secret` references filled in; what it returned, credentials redacted.
pub(crate) async fn mutate(st: &AppState, client: &Client, c: &ScryerMutateConfig) -> Result<Value, String> {
    let m = c.mutation.trim();
    if mutation_rule(m) == MutationRule::Denied {
        return Err(format!("PDT never sends {m}: signing in and the user's own credentials are done in Scryer"));
    }
    let args = match c.args.clone() {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("args must be an object of the mutation's arguments by name".into()),
    };
    let s = client.schema().await;
    let document = mutation_document(&s.schema, m, &args, c.select.as_deref())?;
    // Checked with a stand-in for each secret, so no secret is in an error.
    let stand_in = replace_refs(&Value::Object(args.clone()), &|_| json!("secret"));
    check_document(&s.schema, &document, None, &stand_in, false)?;
    let (vars, sent) = resolve_secrets(st, &Value::Object(args)).await?;
    let data = client.gql_named(&document, vars, Some("PdtMutate")).await.map_err(|e| scrub_text(&e, &sent))?;
    Ok(scrub(&data[m], &sent))
}

fn scrub_text(text: &str, sent: &[String]) -> String {
    sent.iter().filter(|s| s.len() >= 4).fold(text.to_string(), |t, s| t.replace(s.as_str(), REDACTED))
}

/// `v` cut to fit [`MAX_ANSWER`]: the biggest lists halved until it fits, each cut listed.
pub(crate) fn fit(v: Value, max: usize) -> Value {
    fn biggest(v: &Value, path: &str, best: &mut Option<(String, usize)>) {
        match v {
            Value::Array(a) => {
                let size = v.to_string().len();
                if a.len() > 1 && best.as_ref().is_none_or(|(_, s)| size > *s) {
                    *best = Some((path.to_string(), size));
                }
                for (i, x) in a.iter().enumerate() {
                    biggest(x, &format!("{path}/{i}"), best);
                }
            }
            Value::Object(m) => {
                for (k, x) in m {
                    biggest(x, &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")), best);
                }
            }
            _ => {}
        }
    }
    let mut data = v;
    let mut cuts: Vec<(String, usize, usize)> = vec![];
    while data.to_string().len() > max {
        let mut best = None;
        biggest(&data, "", &mut best);
        let Some((path, _)) = best else { break };
        let Some(Value::Array(a)) = data.pointer_mut(&path) else { break };
        let of = a.len();
        a.truncate(of / 2);
        let kept = a.len();
        match cuts.iter_mut().find(|c| c.0 == path) {
            Some(c) => c.1 = kept,
            None => cuts.push((path, kept, of)),
        }
    }
    if cuts.is_empty() && data.to_string().len() <= max {
        return json!({"data": data});
    }
    let truncated: Vec<Value> = cuts
        .into_iter()
        .map(|(p, kept, of)| json!({"path": p.trim_start_matches('/').replace('/', "."), "kept": kept, "of": of}))
        .collect();
    json!({"data": data, "truncated": truncated,
           "hint": "the answer was too long and lists were cut: select fewer fields, or page with the field's \
                    limit/offset (or first/after) arguments"})
}

// ---- the actions ----

fn is_ref(s: &str) -> bool {
    s.contains("{{")
}

/// `scryer_schema`: a tool only.
pub struct SchemaLookup;

impl Action for SchemaLookup {
    type Input = ScryerSchemaArgs;
    type Output = Value;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new("scryer_schema", "Find Scryer's settings and operations in its API")
                .read_only()
                .open_world()
                .pm(),
            title: "Look up Scryer's API",
            description: "Finds and describes Scryer's GraphQL operations and types, to read or change any of its \
                          settings (indexers, download clients, seeding profiles, quality, general settings, jobs): \
                          `search` words in names and descriptions; `operation` one query or mutation with its \
                          arguments, the input types they take and what it returns; `type` one type's fields. With \
                          none, how big the API is.",
        }
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn approval_title(&self, a: &ScryerSchemaArgs) -> String {
        format!("look up Scryer's API ({})", a.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, a: ScryerSchemaArgs) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let s = Client::open(cx.st, &a.connection).await?.schema().await;
            schema_answer(&s, &a)
        })
    }
}

pub struct Query;

impl Action for Query {
    type Input = ScryerQueryConfig;
    type Output = ScryerQueryOutput;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new(
                "scryer_query",
                "Read anything from Scryer's API (settings, indexers, download clients)",
            )
            .read_only()
            .open_world()
            .pm(),
            title: "Read from Scryer",
            description: "Runs a GraphQL query on Scryer (a query document and its variables), checked against \
                          Scryer's schema first; mutations go through scryer_mutate. Credentials in the answer are \
                          redacted, and a long answer is cut (it says where): select fewer fields or page.",
        }
    }

    fn node(&self) -> Option<NodeFace> {
        Some(NodeFace {
            id: "scryer.query",
            title: "Query Scryer",
            description: "Runs a GraphQL query on Scryer (any of its settings and data), checked against Scryer's \
                          schema first: `{{steps.<id>.data}}` is what it answered, credentials redacted.",
            ports: THEN,
        })
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &ScryerQueryConfig) -> Result<(), String> {
        if c.query.trim().is_empty() {
            return Err("query is empty".into());
        }
        if is_ref(&c.query) {
            return Ok(());
        }
        let doc = apollo_compiler::ast::Document::parse(c.query.as_str(), "query.graphql")
            .map_err(|e| format!("the query doesn't parse: {}", messages(&e.errors).join("; ")))?;
        let mutates = doc.definitions.iter().any(|d| {
            matches!(d, apollo_compiler::ast::Definition::OperationDefinition(o)
                if o.operation_type != OperationType::Query)
        });
        if mutates {
            return Err("scryer.query runs queries only: changes are scryer.mutate".into());
        }
        if c.variables.as_ref().is_some_and(crate::core::has_secret_ref) {
            return Err("a query sends no secrets: $secret references go in scryer.mutate".into());
        }
        Ok(())
    }

    fn approval_title(&self, c: &ScryerQueryConfig) -> String {
        format!("query Scryer ({})", c.connection)
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, c: ScryerQueryConfig) -> BoxFut<'a, Result<ScryerQueryOutput, String>> {
        Box::pin(
            async move { Ok(ScryerQueryOutput { data: query(&Client::open(cx.st, &c.connection).await?, &c).await? }) },
        )
    }

    /// A long answer is cut to fit, and says where.
    fn tool_output(&self, out: ScryerQueryOutput) -> Result<Value, String> {
        Ok(fit(out.data, MAX_ANSWER))
    }
}

pub struct Mutate;

impl Action for Mutate {
    type Input = ScryerMutateConfig;
    type Output = ScryerMutateOutput;

    fn tool(&self) -> ToolFace {
        ToolFace {
            decl: ToolDecl::new("scryer_mutate", "Change Scryer's settings through its API").asks().open_world().pm(),
            title: "Change Scryer",
            description: "Runs one GraphQL mutation on Scryer: its name, its arguments by name and the fields to read \
                          back (its scalar fields when left out), checked against Scryer's schema first. Put a \
                          password or API key in as {\"$secret\": \"<connection>\"}: PDT fills in that connection's \
                          secret, so it never passes through you. The user confirms each call unless the Scryer \
                          connection lets you act; deleting, resetting, upgrades and calls that send a secret always \
                          ask.",
        }
    }

    fn node(&self) -> Option<NodeFace> {
        Some(NodeFace {
            id: "scryer.mutate",
            title: "Change Scryer",
            description: "Runs one GraphQL mutation on Scryer (`createIndexerConfig`, `updateGeneralSettings`, \
                          `triggerJob`, …) with its arguments, checked against Scryer's schema first. A credential is \
                          `{\"$secret\": \"<connection>\"}`. `{{steps.<id>.result}}` is what it returned, credentials \
                          redacted.",
            ports: THEN,
        })
    }

    fn connection(&self) -> Option<ConnectionFace> {
        CONNECTION
    }

    fn check(&self, c: &ScryerMutateConfig) -> Result<(), String> {
        let m = c.mutation.trim();
        if m.is_empty() {
            return Err("mutation is empty".into());
        }
        if !is_ref(m) && mutation_rule(m) == MutationRule::Denied {
            return Err(format!("PDT never sends {m}: signing in and the user's own credentials are done in Scryer"));
        }
        if c.args.as_ref().is_some_and(|a| !a.is_null() && !a.is_object()) {
            return Err("args must be an object of the mutation's arguments by name".into());
        }
        Ok(())
    }

    fn approval_title(&self, c: &ScryerMutateConfig) -> String {
        format!("run {} on Scryer ({})", c.mutation.trim(), c.connection)
    }

    /// Deleting, resetting, upgrades and calls that send a secret always ask; other changes ask unless the connection
    /// lets the PM act.
    fn ask_target(&self, args: &Value) -> AskTarget {
        if crate::core::mutate_always_asks(args) {
            return AskTarget::Always;
        }
        AskTarget::Connection { kind: SCRYER, name: args.get("connection").and_then(Value::as_str).map(String::from) }
    }

    fn run<'a>(
        &'a self,
        cx: &'a ActionCx<'a>,
        c: ScryerMutateConfig,
    ) -> BoxFut<'a, Result<ScryerMutateOutput, String>> {
        Box::pin(async move {
            let client = Client::open(cx.st, &c.connection).await?;
            let result = mutate(cx.st, &client, &c).await?;
            let m = c.mutation.trim().to_string();
            if cx.via == Via::Tool {
                tracing::info!(connection = %c.connection, mutation = %m, "the PM changed Scryer");
            }
            Ok(ScryerMutateOutput { mutation: m, result })
        })
    }

    /// A long result is cut to fit, and says where.
    fn tool_output(&self, out: ScryerMutateOutput) -> Result<Value, String> {
        let mut v = fit(out.result, MAX_ANSWER);
        v["mutation"] = json!(out.mutation);
        if let Some(o) = v.as_object_mut()
            && let Some(r) = o.remove("data")
        {
            o.insert("result".into(), r);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Arc<ScryerSchema> {
        bundled()
    }

    #[test]
    fn queries_are_checked_against_the_schema() {
        let s = schema();
        let ok = "query { indexers { id name } }";
        assert!(check_document(&s.schema, ok, None, &Value::Null, true).is_ok());
        let e = check_document(&s.schema, "query { indexers { nope } }", None, &Value::Null, true).unwrap_err();
        assert!(e.contains("nope") && e.contains("scryer_schema"), "{e}");
        let e = check_document(&s.schema, "mutation { triggerRssSync { __typename } }", None, &Value::Null, true)
            .unwrap_err();
        assert!(e.contains("scryer_mutate"), "{e}");
        // A variable of the wrong type.
        let q = "query Q($limit: Int!) { downloadQueuePage(limit: $limit) { totalCount } }";
        let e = check_document(&s.schema, q, None, &json!({"limit": "ten"}), true).unwrap_err();
        assert!(e.contains("variables"), "{e}");
        assert!(check_document(&s.schema, q, None, &json!({"limit": 10}), true).is_ok());
        let two = "query A { indexers { id } } query B { downloadClientConfigs { id } }";
        assert!(check_document(&s.schema, two, None, &Value::Null, true).unwrap_err().contains("operationName"));
        assert!(check_document(&s.schema, two, Some("B"), &Value::Null, true).is_ok());
    }

    #[test]
    fn a_mutation_document_types_its_arguments_from_the_schema() {
        let s = schema();
        let args = json!({"input": {"name": "NZBgeek", "providerType": "newznab"}});
        let d =
            mutation_document(&s.schema, "createIndexerConfig", args.as_object().unwrap(), Some("id name")).unwrap();
        assert_eq!(
            d,
            "mutation PdtMutate($input: CreateIndexerConfigInput!) { createIndexerConfig(input: $input) { id name } }"
        );
        // Left out, the selection is its scalar fields.
        let d = mutation_document(&s.schema, "createIndexerConfig", args.as_object().unwrap(), None).unwrap();
        assert!(d.contains("{ id name providerType") && d.contains("hasApiKey"), "{d}");
        assert!(check_document(&s.schema, &d, None, &args, false).is_ok());
        let e = mutation_document(&s.schema, "createIndexer", &Default::default(), None).unwrap_err();
        assert!(e.contains("createIndexerConfig"), "{e}");
        let e = mutation_document(&s.schema, "createIndexerConfig", json!({"name": "x"}).as_object().unwrap(), None)
            .unwrap_err();
        assert!(e.contains("takes no argument name"), "{e}");
        // A wrong argument type is caught before anything is sent.
        let bad = json!({"input": {"name": 7}});
        let d = mutation_document(&s.schema, "createIndexerConfig", bad.as_object().unwrap(), None).unwrap();
        assert!(check_document(&s.schema, &d, None, &bad, false).unwrap_err().contains("variables"));
    }

    #[test]
    fn schema_search_finds_operations_by_words() {
        let s = schema();
        let found = search_schema(&s.schema, "indexer config");
        let names: Vec<&str> = found["matches"].as_array().unwrap().iter().filter_map(|m| m["name"].as_str()).collect();
        assert!(names.contains(&"createIndexerConfig") && names.contains(&"indexers"), "{names:?}");
        let found = search_schema(&s.schema, "download clients");
        let names: Vec<&str> = found["matches"].as_array().unwrap().iter().filter_map(|m| m["name"].as_str()).collect();
        assert!(names.contains(&"createDownloadClientConfig"), "{names:?}");
        let op = describe_operation(&s.schema, "createIndexerConfig").unwrap();
        assert_eq!(op["kind"], "mutation");
        assert_eq!(op["args"][0]["type"], "CreateIndexerConfigInput!");
        let inputs: Vec<&str> =
            op["inputTypes"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(inputs.contains(&"CreateIndexerConfigInput") && inputs.contains(&"ProviderConfigValueInput"));
        assert_eq!(describe_operation(&s.schema, "deleteIndexerConfig").unwrap()["asks"], "always asks the user");
        let t = describe_type(&s.schema, "MediaFacetValue").unwrap();
        assert!(t["values"].as_array().unwrap().iter().any(|v| v["name"] == "MOVIE"));
        let a = ScryerSchemaArgs { connection: "x".into(), search: None, operation: None, type_name: None };
        let v = schema_answer(&s, &a).unwrap();
        assert!(v["mutations"].as_u64().unwrap() > 200 && v["schema"] == "bundled", "{v}");
    }

    /// An introspection answer becomes the same schema (the fake Scryer answers introspection from the bundled one).
    #[test]
    fn a_live_schema_is_read_from_introspection() {
        let s = schema();
        let doc = ExecutableDocument::parse_and_validate(&s.schema, INTROSPECTION, "i.graphql").unwrap();
        let op = doc.operations.get(None).unwrap();
        let vars = apollo_compiler::request::coerce_variable_values(&s.schema, op, &JsonMap::new()).unwrap();
        let r =
            apollo_compiler::introspection::partial_execute(&s.schema, &s.schema.implementers_map(), &doc, op, &vars)
                .unwrap();
        let data = serde_json::to_value(r.data).unwrap();
        let live = live_schema(&data).unwrap();
        assert_eq!(live.types.len(), s.schema.types.len());
        assert!(describe_operation(&live, "createIndexerConfig").is_some());
        assert!(live_schema(&json!({"__schema": {"types": []}})).is_err());
    }

    #[test]
    fn long_answers_are_cut_and_say_so() {
        let items: Vec<Value> = (0..1000).map(|i| json!({"id": i, "name": format!("title {i}")})).collect();
        let v = fit(json!({"titles": {"items": items, "totalCount": 1000}}), 4096);
        assert!(v.to_string().len() < 4096 + 400);
        assert_eq!(v["truncated"][0]["path"], "titles.items");
        assert_eq!(v["truncated"][0]["of"], 1000);
        assert!(v["hint"].as_str().unwrap().contains("limit"));
        assert_eq!(fit(json!({"a": 1}), 4096), json!({"data": {"a": 1}}));
    }

    #[test]
    fn echoed_secrets_are_scrubbed() {
        let v =
            scrub(&json!({"name": "sab", "note": "connected with hunter22", "password": "x"}), &["hunter22".into()]);
        assert_eq!(v, json!({"name": "sab", "note": format!("connected with {REDACTED}"), "password": REDACTED}));
    }
}
