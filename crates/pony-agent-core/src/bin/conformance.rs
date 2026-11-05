//! 契约 conformance kit 外部入口（ADR 0068 决策 2 的骨架，W2 前置）。
//!
//! 用法：
//! ```text
//! cargo run -p pony-agent-core --bin conformance -- --kit <dir> [--strict] [--schemas <dir>]
//! ```
//!
//! 退出码语义固定（ADR 0068 / task-4 契约）：
//! - `0` 通过：kit 内每个用例的「预期」与「实测」一致；
//! - `2` 用例失败：至少一个用例的预期与实测不一致（含 expect 标错、reason 未命中）；
//! - `3` 配置非法：参数错误、kit 目录/用例文件不可读、用例形状非法、schema 缺失或使用未实现的校验关键字。
//!
//! 用例文件形状（`<kit>/*.json`，非递归）：
//! ```json
//! {
//!   "id": "tool-descriptor/builtin-read",   // 必填，kit 内唯一
//!   "contract": "tool-descriptor",          // 必填，见 CONTRACTS
//!   "expect": "valid",                      // 必填，valid | invalid
//!   "reason": "does not match pattern",     // expect=invalid 时必填：必须命中至少一条实测错误文本
//!   "instance": { }                          // 必填，待校验实例
//! }
//! ```
//!
//! 本骨架只做**纯 schema 校验**：使用 `docs/contracts/*.v1.schema.json` 与下文内置的最小
//! JSON Schema 校验器（draft-07 子集），不引入新依赖。schema 里出现未实现的关键字时
//! **fail-closed 报配置非法（3）**，绝不静默忽略（否则就是拿恒真校验换绿灯）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

const EXIT_OK: u8 = 0;
const EXIT_CASE_FAILED: u8 = 2;
const EXIT_CONFIG_INVALID: u8 = 3;

/// 契约名 → schema 文件名（与 ADR 0068 决策 1 的五份契约一一对应）。
const CONTRACTS: &[(&str, &str)] = &[
    ("tool-descriptor", "tool-descriptor.v1.schema.json"),
    ("hooks", "hooks.v1.schema.json"),
    ("mcp-config", "mcp-config.v1.schema.json"),
    ("skill-manifest", "skill-manifest.v1.schema.json"),
    ("plugin", "plugin.v1.schema.json"),
];

/// 已实现的 JSON Schema 关键字白名单。schema 用到白名单外的关键字即视为配置非法。
const SUPPORTED_KEYWORDS: &[&str] = &[
    "$schema",
    "$id",
    "$comment",
    "$defs",
    "version",
    "title",
    "description",
    "definitions",
    "type",
    "enum",
    "const",
    "required",
    "properties",
    "additionalProperties",
    "items",
    "minItems",
    "maxItems",
    "uniqueItems",
    "minLength",
    "maxLength",
    "pattern",
    "minimum",
    "maximum",
    "minProperties",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "$ref",
];

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("[conformance] 配置非法: {message}");
            ExitCode::from(EXIT_CONFIG_INVALID)
        }
    }
}

#[derive(Debug)]
struct Options {
    kit: PathBuf,
    schemas: Option<PathBuf>,
    strict: bool,
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut kit: Option<PathBuf> = None;
    let mut schemas: Option<PathBuf> = None;
    let mut strict = false;

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--kit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--kit 需要一个目录参数".to_string())?;
                kit = Some(PathBuf::from(value));
            }
            "--schemas" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--schemas 需要一个目录参数".to_string())?;
                schemas = Some(PathBuf::from(value));
            }
            "--strict" => strict = true,
            "-h" | "--help" => {
                print_usage();
                return Ok(None);
            }
            other => return Err(format!("未知参数 `{other}`")),
        }
        index += 1;
    }

    let kit = kit.ok_or_else(|| "缺少必需参数 --kit <dir>".to_string())?;
    Ok(Some(Options {
        kit,
        schemas,
        strict,
    }))
}

fn print_usage() {
    println!(
        "用法: conformance --kit <dir> [--strict] [--schemas <dir>]\n\
         \n\
         退出码: 0 通过 / 2 用例失败 / 3 配置非法\n\
         \n\
         --kit     用例目录（读取其中的 *.json，非递归）\n\
         --schemas 契约 schema 目录（缺省：从当前目录向上查找 docs/contracts）\n\
         --strict  严格模式：用例文件出现未知字段即配置非法"
    );
}

#[derive(Debug)]
struct CaseOutcome {
    id: String,
    passed: bool,
    detail: String,
}

fn run() -> Result<u8, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&raw)? {
        Some(options) => options,
        None => return Ok(EXIT_OK),
    };

    let schemas_dir = match options.schemas.clone() {
        Some(dir) => dir,
        None => discover_schemas_dir()
            .ok_or_else(|| "未找到 docs/contracts 目录，请用 --schemas 显式指定".to_string())?,
    };
    let schemas = load_schemas(&schemas_dir)?;
    for (contract, doc) in &schemas {
        println!("[conformance] schema {contract}={}", doc.version);
    }

    let mut cases = load_cases(&options)?;
    if cases.is_empty() {
        return Err(format!(
            "kit 目录 `{}` 内没有 *.json 用例文件（空 kit 不构成通过证据）",
            options.kit.display()
        ));
    }

    let mut seen_ids: BTreeMap<String, usize> = BTreeMap::new();
    for (index, case) in cases.iter().enumerate() {
        if let Some(first) = seen_ids.insert(case.id.clone(), index) {
            return Err(format!(
                "用例 id `{}` 在 kit 内重复（第 {} 与第 {} 个文件）",
                case.id,
                first + 1,
                index + 1
            ));
        }
    }

    let mut outcomes: Vec<CaseOutcome> = Vec::with_capacity(cases.len());
    for case in cases.drain(..) {
        let schema = schemas
            .get(case.contract.as_str())
            .ok_or_else(|| format!("用例 `{}` 引用了未知契约 `{}`", case.id, case.contract))?;
        let validator = Validator { root: &schema.root };
        let mut failures: Vec<Failure> = Vec::new();
        validator.validate(&schema.root, &case.instance, "/", &mut failures);
        let summary = summarize(&failures);

        let passed = if case.expect == "valid" {
            failures.is_empty()
        } else {
            // 负例必须真正被拒，且失败原因与用例声明的原因一致（逐例断言，禁止整批笼统失败）。
            !failures.is_empty() && summary.contains(case.reason.as_str())
        };
        let detail = if passed {
            "ok".to_string()
        } else if case.expect == "valid" {
            format!("期望通过但被拒: {summary}")
        } else if failures.is_empty() {
            "期望被拒但校验通过".to_string()
        } else {
            format!(
                "被拒但原因不匹配（期望包含 `{}`）: {summary}",
                case.reason
            )
        };
        outcomes.push(CaseOutcome {
            id: format!("{} [{}]", case.id, case.contract),
            passed,
            detail,
        });
    }

    let mut failed = 0usize;
    for outcome in &outcomes {
        if outcome.passed {
            println!("PASS {}", outcome.id);
        } else {
            failed += 1;
            println!("FAIL {} :: {}", outcome.id, outcome.detail);
        }
    }
    println!(
        "[conformance] cases={} passed={} failed={} schemas={} strict={}",
        outcomes.len(),
        outcomes.len() - failed,
        failed,
        schemas.len(),
        options.strict
    );

    if failed > 0 {
        Ok(EXIT_CASE_FAILED)
    } else {
        Ok(EXIT_OK)
    }
}

/// 从当前目录逐级向上查找 `docs/contracts`（含五份契约 schema 的目录）。
fn discover_schemas_dir() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("PONY_CONTRACTS_DIR") {
        let path = PathBuf::from(explicit);
        if path.is_dir() {
            return Some(path);
        }
    }
    let cwd = std::env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join("docs").join("contracts");
        if candidate.join("tool-descriptor.v1.schema.json").is_file() {
            return Some(candidate);
        }
    }
    None
}

struct SchemaDoc {
    version: String,
    root: Value,
}

fn load_schemas(dir: &Path) -> Result<BTreeMap<String, SchemaDoc>, String> {
    if !dir.is_dir() {
        return Err(format!("schema 目录不存在: {}", dir.display()));
    }
    let mut map = BTreeMap::new();
    for (contract, file) in CONTRACTS {
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("读取 schema 失败 {}: {err}", path.display()))?;
        let root: Value = serde_json::from_str(&text)
            .map_err(|err| format!("schema 不是合法 JSON {}: {err}", path.display()))?;
        let version = root
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("schema 缺少 version 字段: {}", path.display()))?
            .to_string();
        if root.get("$id").and_then(Value::as_str).is_none() {
            return Err(format!("schema 缺少 $id 字段: {}", path.display()));
        }
        let validator = Validator { root: &root };
        validator
            .check_supported_keywords(&root, "/")
            .map_err(|err| format!("{}: {err}", path.display()))?;
        map.insert(contract.to_string(), SchemaDoc { version, root });
    }
    Ok(map)
}

struct Case {
    id: String,
    contract: String,
    expect: String,
    reason: String,
    instance: Value,
}

fn load_cases(options: &Options) -> Result<Vec<Case>, String> {
    if !options.kit.is_dir() {
        return Err(format!("kit 目录不存在: {}", options.kit.display()));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&options.kit)
        .map_err(|err| format!("读取 kit 目录失败 {}: {err}", options.kit.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    files.sort();

    let mut cases = Vec::with_capacity(files.len());
    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("读取用例失败 {}: {err}", path.display()))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| format!("用例不是合法 JSON {}: {err}", path.display()))?;
        cases.push(parse_case(&path, &value, options.strict)?);
    }
    Ok(cases)
}

fn parse_case(path: &Path, value: &Value, strict: bool) -> Result<Case, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("用例根必须是 JSON object: {}", path.display()))?;
    if strict {
        for key in object.keys() {
            if !matches!(key.as_str(), "id" | "contract" | "expect" | "reason" | "instance" | "note")
            {
                return Err(format!(
                    "strict 模式拒绝未知用例字段 `{key}`: {}",
                    path.display()
                ));
            }
        }
    }
    let string_field = |name: &str| -> Result<String, String> {
        object
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("用例缺少字符串字段 `{name}`: {}", path.display()))
    };

    let id = string_field("id")?;
    if id.trim().is_empty() {
        return Err(format!("用例 id 不能为空: {}", path.display()));
    }
    let contract = string_field("contract")?;
    if !CONTRACTS.iter().any(|(name, _)| *name == contract) {
        return Err(format!(
            "用例 `{id}` 引用了未知契约 `{contract}`（可选: {}）",
            CONTRACTS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(" / ")
        ));
    }
    let expect = string_field("expect")?;
    if expect != "valid" && expect != "invalid" {
        return Err(format!(
            "用例 `{id}` 的 expect 只能为 valid 或 invalid，实得 `{expect}`"
        ));
    }
    let reason = object
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if expect == "invalid" && reason.trim().is_empty() {
        return Err(format!(
            "负例 `{id}` 必须声明 reason（用于逐例断言失败原因，禁止整批笼统失败）"
        ));
    }
    let instance = object
        .get("instance")
        .cloned()
        .ok_or_else(|| format!("用例 `{id}` 缺少 instance"))?;

    Ok(Case {
        id,
        contract,
        expect,
        reason,
        instance,
    })
}

#[derive(Debug)]
struct Failure {
    path: String,
    message: String,
}

fn summarize(failures: &[Failure]) -> String {
    failures
        .iter()
        .map(|failure| format!("{}: {}", failure.path, failure.message))
        .collect::<Vec<_>>()
        .join("; ")
}

struct Validator<'a> {
    root: &'a Value,
}

impl<'a> Validator<'a> {
    /// fail-closed：schema 用了白名单外的关键字就直接报配置非法。
    fn check_supported_keywords(&self, schema: &Value, path: &str) -> Result<(), String> {
        let object = match schema.as_object() {
            Some(object) => object,
            None => return Ok(()),
        };
        for (key, value) in object {
            if !SUPPORTED_KEYWORDS.contains(&key.as_str()) {
                return Err(format!(
                    "schema 使用未实现的 JSON Schema 关键字 `{key}`（位置 {path}），拒绝给出恒真校验"
                ));
            }
            let child_path = format!("{path}{key}");
            match key.as_str() {
                "properties" | "definitions" | "$defs" => {
                    if let Some(map) = value.as_object() {
                        for (name, child) in map {
                            self.check_supported_keywords(child, &format!("{child_path}/{name}"))?;
                        }
                    }
                }
                "items" | "additionalProperties" | "not" | "if" | "then" | "else" | "contains" => {
                    if value.is_object() {
                        self.check_supported_keywords(value, &child_path)?;
                    }
                }
                "allOf" | "anyOf" | "oneOf" => {
                    if let Some(items) = value.as_array() {
                        for (index, child) in items.iter().enumerate() {
                            self.check_supported_keywords(child, &format!("{child_path}/{index}"))?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn resolve_ref(&self, reference: &str) -> Option<Value> {
        let pointer = reference.strip_prefix('#')?;
        self.root.pointer(pointer).cloned()
    }

    fn validate(&self, schema: &Value, instance: &Value, path: &str, out: &mut Vec<Failure>) {
        let object = match schema.as_object() {
            Some(object) => object,
            None => {
                // `true`/`false` 作为 schema 的写法不在 v1 契约中使用。
                return;
            }
        };

        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            match self.resolve_ref(reference) {
                Some(target) => {
                    self.validate(&target, instance, path, out);
                    return;
                }
                None => {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("unresolvable $ref {reference}"),
                    });
                    return;
                }
            }
        }

        if let Some(expected) = object.get("type") {
            if !matches_type(expected, instance) {
                out.push(Failure {
                    path: path.to_string(),
                    message: format!("expected type {}, got {}", describe_type(expected), actual_type(instance)),
                });
                return;
            }
        }

        if let Some(allowed) = object.get("enum").and_then(Value::as_array) {
            if !allowed.iter().any(|candidate| candidate == instance) {
                let list = allowed
                    .iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(Failure {
                    path: path.to_string(),
                    message: format!("value is not one of [{list}]"),
                });
            }
        }

        if let Some(expected) = object.get("const") {
            if expected != instance {
                out.push(Failure {
                    path: path.to_string(),
                    message: format!("value does not equal const {expected}"),
                });
            }
        }

        if let Some(text) = instance.as_str() {
            if let Some(limit) = object.get("minLength").and_then(Value::as_u64) {
                if (text.chars().count() as u64) < limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("string is shorter than {limit}"),
                    });
                }
            }
            if let Some(limit) = object.get("maxLength").and_then(Value::as_u64) {
                if (text.chars().count() as u64) > limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("string is longer than {limit}"),
                    });
                }
            }
            if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
                match regex::Regex::new(pattern) {
                    Ok(regex) => {
                        if !regex.is_match(text) {
                            out.push(Failure {
                                path: path.to_string(),
                                message: format!("string does not match pattern {pattern}"),
                            });
                        }
                    }
                    Err(err) => out.push(Failure {
                        path: path.to_string(),
                        message: format!("invalid pattern {pattern}: {err}"),
                    }),
                }
            }
        }

        if let Some(number) = instance.as_f64() {
            if let Some(limit) = object.get("minimum").and_then(Value::as_f64) {
                if number < limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("number is less than {limit}"),
                    });
                }
            }
            if let Some(limit) = object.get("maximum").and_then(Value::as_f64) {
                if number > limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("number is greater than {limit}"),
                    });
                }
            }
        }

        if let Some(map) = instance.as_object() {
            if let Some(required) = object.get("required").and_then(Value::as_array) {
                for name in required.iter().filter_map(Value::as_str) {
                    if !map.contains_key(name) {
                        out.push(Failure {
                            path: path.to_string(),
                            message: format!("missing required field '{name}'"),
                        });
                    }
                }
            }
            if let Some(limit) = object.get("minProperties").and_then(Value::as_u64) {
                if (map.len() as u64) < limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("object has fewer than {limit} properties"),
                    });
                }
            }
            let properties = object.get("properties").and_then(Value::as_object);
            if let Some(properties) = properties {
                for (key, subschema) in properties {
                    if let Some(value) = map.get(key) {
                        self.validate(subschema, value, &format!("{path}{key}"), out);
                    }
                }
            }
            if let Some(additional) = object.get("additionalProperties") {
                let known: Vec<&String> = properties
                    .map(|properties| properties.keys().collect())
                    .unwrap_or_default();
                for (key, value) in map {
                    if known.iter().any(|known_key| *known_key == key) {
                        continue;
                    }
                    match additional {
                        Value::Bool(false) => out.push(Failure {
                            path: path.to_string(),
                            message: format!("unexpected additional field '{key}'"),
                        }),
                        Value::Object(_) => {
                            self.validate(additional, value, &format!("{path}{key}"), out)
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Some(items) = instance.as_array() {
            if let Some(limit) = object.get("minItems").and_then(Value::as_u64) {
                if (items.len() as u64) < limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("array has fewer than {limit} items"),
                    });
                }
            }
            if let Some(limit) = object.get("maxItems").and_then(Value::as_u64) {
                if (items.len() as u64) > limit {
                    out.push(Failure {
                        path: path.to_string(),
                        message: format!("array has more than {limit} items"),
                    });
                }
            }
            if object.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
                let mut seen: Vec<String> = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let rendered = item.to_string();
                    if seen.contains(&rendered) {
                        out.push(Failure {
                            path: path.to_string(),
                            message: format!("array items are not unique at index {index}"),
                        });
                        break;
                    }
                    seen.push(rendered);
                }
            }
            if let Some(item_schema) = object.get("items") {
                for (index, item) in items.iter().enumerate() {
                    self.validate(item_schema, item, &format!("{path}{index}"), out);
                }
            }
            if let Some(contains) = object.get("contains") {
                let mut any = false;
                for item in items {
                    let mut probe: Vec<Failure> = Vec::new();
                    self.validate(contains, item, path, &mut probe);
                    if probe.is_empty() {
                        any = true;
                        break;
                    }
                }
                if !any {
                    out.push(Failure {
                        path: path.to_string(),
                        message: "array does not contain a matching item".to_string(),
                    });
                }
            }
        }

        if let Some(branches) = object.get("allOf").and_then(Value::as_array) {
            for branch in branches {
                self.validate(branch, instance, path, out);
            }
        }

        if let Some(branches) = object.get("anyOf").and_then(Value::as_array) {
            let mut matched = 0usize;
            for branch in branches {
                let mut probe: Vec<Failure> = Vec::new();
                self.validate(branch, instance, path, &mut probe);
                if probe.is_empty() {
                    matched += 1;
                }
            }
            if matched == 0 {
                out.push(Failure {
                    path: path.to_string(),
                    message: format!(
                        "must match at least one of the {} anyOf branches",
                        branches.len()
                    ),
                });
            }
        }

        if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
            let mut matched = 0usize;
            for branch in branches {
                let mut probe: Vec<Failure> = Vec::new();
                self.validate(branch, instance, path, &mut probe);
                if probe.is_empty() {
                    matched += 1;
                }
            }
            if matched != 1 {
                out.push(Failure {
                    path: path.to_string(),
                    message: format!(
                        "must match exactly one of the {} oneOf branches (matched {matched})",
                        branches.len()
                    ),
                });
            }
        }

        if let Some(excluded) = object.get("not") {
            let mut probe: Vec<Failure> = Vec::new();
            self.validate(excluded, instance, path, &mut probe);
            if probe.is_empty() {
                out.push(Failure {
                    path: path.to_string(),
                    message: "must not match the excluded schema".to_string(),
                });
            }
        }

        if let Some(condition) = object.get("if") {
            let mut probe: Vec<Failure> = Vec::new();
            self.validate(condition, instance, path, &mut probe);
            let branch = if probe.is_empty() {
                object.get("then")
            } else {
                object.get("else")
            };
            if let Some(branch) = branch {
                self.validate(branch, instance, path, out);
            }
        }
    }
}

fn describe_type(expected: &Value) -> String {
    match expected {
        Value::String(name) => name.clone(),
        Value::Array(names) => names
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join("|"),
        other => other.to_string(),
    }
}

fn actual_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn matches_type(expected: &Value, instance: &Value) -> bool {
    match expected {
        Value::String(name) => matches_single_type(name, instance),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| matches_single_type(name, instance)),
        _ => true,
    }
}

fn matches_single_type(name: &str, instance: &Value) -> bool {
    match name {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        "integer" => instance
            .as_f64()
            .map(|value| value.fract() == 0.0)
            .unwrap_or(false),
        "number" => instance.is_number(),
        _ => true,
    }
}
