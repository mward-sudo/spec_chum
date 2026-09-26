//! Keep the published `OpenAPI` document in sync with Axum's route table.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agent_server::routes::{router, AppState};
use axum::{body::Body, http::Request, http::StatusCode};
use control_plane::ControlPlane;
use serde_json::Value;
use spec_chum_host::ModelId;
use syn::visit::Visit;
use syn::{Expr, ExprCall, ExprMethodCall, Fields, Item, Lit, Type};
use tower::ServiceExt;

const OPENAPI: &str = include_str!("../openapi.json");

#[derive(Default)]
struct RouteCollector {
    routes: BTreeMap<String, BTreeSet<String>>,
}

impl<'ast> Visit<'ast> for RouteCollector {
    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if node.method == "route" {
            if let (Some(Expr::Lit(path)), Some(service)) =
                (node.args.first(), node.args.iter().nth(1))
            {
                if let Lit::Str(path) = &path.lit {
                    let mut methods = BTreeSet::new();
                    collect_http_methods(service, &mut methods);
                    self.routes.insert(path.value(), methods);
                }
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn collect_http_methods(expr: &Expr, methods: &mut BTreeSet<String>) {
    const HTTP_METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];
    match expr {
        Expr::Call(ExprCall { func, args, .. }) => {
            if let Expr::Path(path) = func.as_ref() {
                if let Some(method) = path.path.get_ident().map(ToString::to_string) {
                    if HTTP_METHODS.contains(&method.as_str()) {
                        methods.insert(method);
                    }
                }
            }
            for arg in args {
                collect_http_methods(arg, methods);
            }
        }
        Expr::MethodCall(call) => {
            let method = call.method.to_string();
            if HTTP_METHODS.contains(&method.as_str()) {
                methods.insert(method);
            }
            collect_http_methods(&call.receiver, methods);
            for arg in &call.args {
                collect_http_methods(arg, methods);
            }
        }
        _ => {}
    }
}

fn implementation_routes() -> BTreeMap<String, BTreeSet<String>> {
    let source = include_str!("../src/routes/mod.rs");
    let parsed = syn::parse_file(source).expect("parse route source");
    let router = parsed
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "router" => Some(function),
            _ => None,
        })
        .expect("router function exists");
    let mut collector = RouteCollector::default();
    collector.visit_item_fn(router);
    collector.routes
}

#[test]
fn openapi_methods_match_axum_route_table() {
    let spec: Value = serde_json::from_str(OPENAPI).expect("valid OpenAPI JSON");
    assert_eq!(spec["openapi"], "3.0.3");
    let paths = spec["paths"].as_object().expect("OpenAPI paths object");
    let documented: BTreeMap<String, BTreeSet<String>> = paths
        .iter()
        .map(|(path, operations)| {
            let methods = operations
                .as_object()
                .expect("path item object")
                .keys()
                .cloned()
                .collect();
            (path.clone(), methods)
        })
        .collect();
    assert_eq!(documented, implementation_routes());
    for (path, methods) in &documented {
        let placeholders: BTreeSet<&str> = path
            .split('{')
            .skip(1)
            .filter_map(|segment| segment.split('}').next())
            .collect();
        for method in methods {
            let parameters = spec["paths"][path][method]["parameters"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let path_parameters: BTreeSet<&str> = parameters
                .iter()
                .filter(|parameter| parameter["in"] == "path")
                .map(|parameter| parameter["name"].as_str().expect("path parameter name"))
                .collect();
            assert_eq!(
                path_parameters, placeholders,
                "OpenAPI path parameters drifted for {method} {path}"
            );
            for parameter in parameters
                .iter()
                .filter(|parameter| parameter["in"] == "path")
            {
                assert_eq!(
                    parameter["required"], true,
                    "path parameter must be required for {method} {path}"
                );
            }
        }
    }
}

#[test]
fn query_dtos_match_openapi_query_parameter_names_types_and_requiredness() {
    let spec: Value = serde_json::from_str(OPENAPI).expect("valid OpenAPI JSON");
    let routes = [
        ("FramebufferQuery", "/v1/framebuffer"),
        ("HostDisplayQuery", "/v1/host/display"),
        ("PeekQuery", "/v1/peek"),
        ("DisasmQuery", "/v1/disasm"),
        ("TraceQuery", "/v1/trace"),
    ];
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(route_dir()).expect("read route source directory") {
        let path = entry.expect("route source entry").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            found.extend(extractor_types(&path, "Query"));
        }
    }
    assert_eq!(
        found,
        routes.iter().map(|(name, _)| (*name).to_owned()).collect(),
        "update query DTO route mappings when Query<T> handlers change"
    );

    for (type_name, path) in routes {
        let (fields, required) = rust_struct_shape(type_name)
            .unwrap_or_else(|| panic!("no Rust query struct found: {type_name}"));
        let component = &spec["components"]["schemas"][type_name];
        assert_eq!(component["x-rust-type"], type_name);
        let parameters = spec["paths"][path]["get"]["parameters"]
            .as_array()
            .expect("OpenAPI query parameters");
        let query_parameters: BTreeMap<String, &Value> = parameters
            .iter()
            .filter(|parameter| parameter["in"] == "query")
            .map(|parameter| {
                (
                    parameter["name"]
                        .as_str()
                        .expect("query parameter name")
                        .to_owned(),
                    parameter,
                )
            })
            .collect();
        assert_eq!(
            query_parameters.keys().cloned().collect::<BTreeSet<_>>(),
            fields.keys().cloned().collect(),
            "query parameter names drifted for {type_name}"
        );
        for (name, rust_type) in fields {
            let parameter = query_parameters[&name];
            assert_eq!(
                parameter["schema"]["type"],
                rust_json_type(&rust_type),
                "query parameter type drifted for {type_name}.{name}"
            );
            assert_eq!(
                parameter["required"],
                required.contains(&name),
                "query parameter requiredness drifted for {type_name}.{name}"
            );
        }
    }
}

#[test]
fn request_and_typed_response_schemas_match_rust_struct_fields() {
    let spec: Value = serde_json::from_str(OPENAPI).expect("valid OpenAPI JSON");
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("OpenAPI schemas");
    let rust_types = schema_rust_types(schemas);
    let mut request_types = BTreeSet::new();
    for entry in std::fs::read_dir(route_dir()).expect("read route source directory") {
        let path = entry.expect("route source entry").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            request_types.extend(json_request_types(&path));
        }
    }
    assert!(
        request_types.is_subset(&rust_types),
        "request structs missing schema components: {:?}",
        request_types.difference(&rust_types).collect::<Vec<_>>()
    );
    assert!(
        [
            "HealthResponse",
            "StatusResponse",
            "LastBreakResponse",
            "HardwareStatusResponse"
        ]
        .iter()
        .all(|name| rust_types.contains(*name)),
        "representative typed response components must stay tied to Rust structs"
    );

    for (schema_name, rust_type) in schemas.iter().filter_map(|(name, schema)| {
        schema["x-rust-type"]
            .as_str()
            .map(|rust_type| (name, rust_type))
    }) {
        let (fields, required) = rust_struct_shape(rust_type).unwrap_or_else(|| {
            panic!("no Rust struct found for schema {schema_name}: {rust_type}")
        });
        let schema_properties = schemas[schema_name]["properties"]
            .as_object()
            .expect("component properties");
        let schema_field_names: BTreeSet<String> = schema_properties.keys().cloned().collect();
        assert_eq!(
            schema_field_names,
            fields.keys().cloned().collect(),
            "OpenAPI properties drifted from Rust struct {rust_type}"
        );
        for (field, rust_type) in &fields {
            let property = &schema_properties[field];
            let base_type = rust_container_item_type(rust_type).unwrap_or(rust_type);
            let expected = if rust_type.starts_with("Vec<") {
                "array"
            } else if rust_types.contains(base_type) {
                "object"
            } else {
                rust_json_type(rust_type)
            };
            if expected == "object" {
                assert!(
                    property["$ref"].is_string() || property["oneOf"].is_array(),
                    "OpenAPI object field {schema_name}.{field} needs a schema reference"
                );
            } else {
                assert_eq!(
                    property["type"].as_str(),
                    Some(expected),
                    "OpenAPI type for {schema_name}.{field} drifted from Rust type {rust_type}"
                );
            }
            if let Some(item_type) = rust_type
                .strip_prefix("Vec<")
                .and_then(|inner| inner.strip_suffix('>'))
            {
                if rust_types.contains(item_type) {
                    assert_eq!(
                        property["items"]["$ref"],
                        format!("#/components/schemas/{item_type}"),
                        "OpenAPI array item for {schema_name}.{field} drifted"
                    );
                }
            }
        }
        let schema_required: BTreeSet<String> = schemas[schema_name]["required"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|field| field.as_str().expect("required property").to_owned())
            .collect();
        assert_eq!(
            schema_required, required,
            "OpenAPI required fields drifted from Rust struct {rust_type}"
        );
    }
}

fn route_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes")
}

fn json_request_types(path: &Path) -> BTreeSet<String> {
    extractor_types(path, "Json")
}

fn extractor_types(path: &Path, wrapper: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(path).expect("read route source");
    let parsed = syn::parse_file(&text).expect("parse route source");
    let mut found = BTreeSet::new();
    for item in parsed.items {
        let Item::Fn(function) = item else { continue };
        for argument in function.sig.inputs {
            let syn::FnArg::Typed(argument) = argument else {
                continue;
            };
            let Type::Path(type_path) = argument.ty.as_ref() else {
                continue;
            };
            let Some(segment) = type_path.path.segments.last() else {
                continue;
            };
            if segment.ident != wrapper {
                continue;
            }
            let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                continue;
            };
            for arg in &args.args {
                if let syn::GenericArgument::Type(Type::Path(request_type)) = arg {
                    if let Some(name) = request_type.path.segments.last() {
                        found.insert(name.ident.to_string());
                    }
                }
            }
        }
    }
    found
}

fn schema_rust_types(schemas: &serde_json::Map<String, Value>) -> BTreeSet<String> {
    schemas
        .values()
        .filter_map(|schema| schema["x-rust-type"].as_str().map(str::to_owned))
        .collect()
}

fn rust_struct_shape(type_name: &str) -> Option<(BTreeMap<String, String>, BTreeSet<String>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?;
    let candidate_paths = [
        root.join("agent_server/src/routes"),
        root.join("control_plane/src"),
        root.join("host_api/src"),
    ];
    for directory in candidate_paths {
        for path in rust_sources(&directory) {
            let text = std::fs::read_to_string(&path).ok()?;
            let parsed = syn::parse_file(&text).ok()?;
            for item in parsed.items {
                let Item::Struct(structure) = item else {
                    continue;
                };
                if structure.ident != type_name {
                    continue;
                }
                let Fields::Named(fields) = structure.fields else {
                    return None;
                };
                let deserializes = structure.attrs.iter().any(|attribute| {
                    attribute.path().is_ident("derive")
                        && matches!(&attribute.meta, syn::Meta::List(list) if list.tokens.to_string().contains("Deserialize"))
                });
                let mut all = BTreeMap::new();
                let mut required = BTreeSet::new();
                for field in fields.named {
                    let name = field.ident?.to_string();
                    let defaulted = field.attrs.iter().any(|attribute| {
                        attribute.path().is_ident("serde")
                            && matches!(&attribute.meta, syn::Meta::List(list) if {
                                let options = list.tokens.to_string();
                                options.contains("default") || options.contains("skip_serializing_if")
                            })
                    });
                    let optional = deserializes
                        && matches!(&field.ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Option"));
                    all.insert(name.clone(), quote_type(&field.ty));
                    if !defaulted && !optional {
                        required.insert(name);
                    }
                }
                return Some((all, required));
            }
        }
    }
    None
}

fn quote_type(ty: &Type) -> String {
    match ty {
        Type::Path(path) => {
            let segment = path.path.segments.last().expect("type path segment");
            let name = segment.ident.to_string();
            if matches!(name.as_str(), "Option" | "Vec") {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        return format!("{name}<{}>", quote_type(inner));
                    }
                }
            }
            name
        }
        Type::Reference(reference) => quote_type(&reference.elem),
        _ => panic!("unsupported request/response field type"),
    }
}

fn rust_json_type(rust_type: &str) -> &'static str {
    let base = rust_type
        .strip_prefix("Option<")
        .and_then(|inner| inner.strip_suffix('>'))
        .or_else(|| {
            rust_type
                .strip_prefix("Vec<")
                .and_then(|inner| inner.strip_suffix('>'))
        })
        .unwrap_or(rust_type);
    match base {
        "bool" => "boolean",
        "String" | "str" => "string",
        "f32" | "f64" => "number",
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
            "integer"
        }
        _ if rust_type.starts_with("Vec<") => "array",
        _ => "string", // Serde enums are represented as strings in these API DTOs.
    }
}

fn rust_container_item_type(rust_type: &str) -> Option<&str> {
    ["Vec<", "Option<"]
        .iter()
        .find_map(|prefix| rust_type.strip_prefix(prefix)?.strip_suffix('>'))
}

fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            sources.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources
}

#[tokio::test]
async fn documented_health_and_error_shapes_match_responses() {
    let spec: Value = serde_json::from_str(OPENAPI).expect("valid OpenAPI JSON");
    let plane = std::sync::Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    plane
        .load_rom_bytes(&vec![0; 16 * 1024])
        .expect("load synthetic 16 KiB test ROM");
    let app = router(AppState {
        plane,
        token: Some("contract-test-token".to_owned()),
        insecure: false,
    });

    let published = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("OpenAPI request"),
        )
        .await
        .expect("OpenAPI handler");
    assert_eq!(published.status(), StatusCode::OK);
    let published_body = axum::body::to_bytes(published.into_body(), usize::MAX)
        .await
        .expect("OpenAPI response body");
    assert_eq!(published_body.as_ref(), OPENAPI.as_bytes());

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .header("authorization", "Bearer contract-test-token")
                .body(Body::empty())
                .expect("health request"),
        )
        .await
        .expect("health handler");
    assert_eq!(health.status(), StatusCode::OK);
    let health_body = axum::body::to_bytes(health.into_body(), usize::MAX)
        .await
        .expect("health response body");
    let health_json: Value = serde_json::from_slice(&health_body).expect("health JSON");
    assert_matches_schema(
        &health_json,
        &spec["components"]["schemas"]["HealthResponse"],
    );

    let inspect = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/inspect")
                .header("authorization", "Bearer contract-test-token")
                .body(Body::empty())
                .expect("inspect request"),
        )
        .await
        .expect("inspect handler");
    assert_eq!(inspect.status(), StatusCode::OK);
    let inspect_body = axum::body::to_bytes(inspect.into_body(), usize::MAX)
        .await
        .expect("inspect response body");
    let inspect_json: Value = serde_json::from_slice(&inspect_body).expect("inspect JSON");
    assert_value_matches_schema(
        &inspect_json,
        &spec["components"]["schemas"]["InspectSnapshot"],
        &spec["components"]["schemas"],
        "InspectSnapshot",
    );
    assert_eq!(
        spec["paths"]["/v1/inspect"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/InspectSnapshot"
    );

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .expect("unauthorized request"),
        )
        .await
        .expect("unauthorized handler");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body = axum::body::to_bytes(unauthorized.into_body(), usize::MAX)
        .await
        .expect("unauthorized response body");
    let unauthorized_json: Value =
        serde_json::from_slice(&unauthorized_body).expect("unauthorized JSON");
    assert_matches_schema(
        &unauthorized_json,
        &spec["components"]["schemas"]["ErrorBody"],
    );
    assert_eq!(
        spec["paths"]["/v1/health"]["get"]["responses"]["401"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/ErrorBody"
    );

    let invalid = app
        .oneshot(
            Request::builder()
                .uri("/v1/peek?addr=not-a-number")
                .header("authorization", "Bearer contract-test-token")
                .body(Body::empty())
                .expect("invalid peek request"),
        )
        .await
        .expect("peek handler");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let error_body = axum::body::to_bytes(invalid.into_body(), usize::MAX)
        .await
        .expect("error response body");
    let error_json: Value = serde_json::from_slice(&error_body).expect("error JSON");
    assert_matches_schema(&error_json, &spec["components"]["schemas"]["ErrorBody"]);
    assert_eq!(
        spec["paths"]["/v1/peek"]["get"]["responses"]["400"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/ErrorBody"
    );
}

fn assert_matches_schema(value: &Value, schema: &Value) {
    let required = schema["required"].as_array().expect("required array");
    let properties = schema["properties"].as_object().expect("schema properties");
    for field in required {
        let field = field.as_str().expect("required field name");
        let actual = value
            .get(field)
            .unwrap_or_else(|| panic!("missing field {field}: {value}"));
        let expected_type = properties[field]["type"].as_str().expect("field type");
        let matches = match expected_type {
            "boolean" => actual.is_boolean(),
            "integer" => actual.is_i64() || actual.is_u64(),
            "number" => actual.is_number(),
            "string" => actual.is_string(),
            "object" => actual.is_object(),
            "array" => actual.is_array(),
            other => panic!("unsupported schema type {other}"),
        };
        assert!(matches, "field {field} has wrong type: {actual}");
    }
}

fn assert_value_matches_schema(value: &Value, schema: &Value, schemas: &Value, context: &str) {
    if value.is_null() {
        assert_eq!(schema["nullable"], true, "{context} is unexpectedly null");
        return;
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .expect("local component schema reference");
        assert_value_matches_schema(value, &schemas[name], schemas, context);
        return;
    }

    let expected_type = schema["type"].as_str().expect("schema type");
    let matches = match expected_type {
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        other => panic!("unsupported schema type {other} at {context}"),
    };
    assert!(matches, "{context} expected {expected_type}, got {value}");

    match expected_type {
        "object" => {
            let object = value.as_object().expect("object value");
            if schema["additionalProperties"] == false {
                let actual: BTreeSet<_> = object.keys().map(String::as_str).collect();
                let expected: BTreeSet<_> = schema["properties"]
                    .as_object()
                    .expect("schema properties")
                    .keys()
                    .map(String::as_str)
                    .collect();
                assert_eq!(actual, expected, "{context} property set drifted");
            }
            let properties = schema["properties"].as_object().expect("schema properties");
            for field in schema["required"].as_array().expect("required fields") {
                let field = field.as_str().expect("required field name");
                let child = object
                    .get(field)
                    .unwrap_or_else(|| panic!("{context} missing required field {field}"));
                assert_value_matches_schema(
                    child,
                    &properties[field],
                    schemas,
                    &format!("{context}.{field}"),
                );
            }
            for (field, child_schema) in properties {
                if let Some(child) = object.get(field) {
                    assert_value_matches_schema(
                        child,
                        child_schema,
                        schemas,
                        &format!("{context}.{field}"),
                    );
                }
            }
        }
        "array" => {
            let item_schema = &schema["items"];
            for (index, item) in value.as_array().expect("array value").iter().enumerate() {
                assert_value_matches_schema(
                    item,
                    item_schema,
                    schemas,
                    &format!("{context}[{index}]"),
                );
            }
        }
        _ => {}
    }
}
