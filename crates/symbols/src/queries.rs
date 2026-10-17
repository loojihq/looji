//! The tree-sitter queries that find symbols, one per language. Each match captures the definition's name as `@name`,
//! and the whole definition as `@definition.<kind>`, where the kind is `function`, `type`, or `module`. The line range
//! is the definition's, so a function's range covers its body.

pub const RUST: &str = r#"
(function_item name: (identifier) @name) @definition.function
(function_signature_item name: (identifier) @name) @definition.function
(struct_item name: (type_identifier) @name) @definition.type
(enum_item name: (type_identifier) @name) @definition.type
(union_item name: (type_identifier) @name) @definition.type
(trait_item name: (type_identifier) @name) @definition.type
(type_item name: (type_identifier) @name) @definition.type
(mod_item name: (identifier) @name) @definition.module
"#;

pub const PYTHON: &str = r#"
(function_definition name: (identifier) @name) @definition.function
(class_definition name: (identifier) @name) @definition.type
"#;

pub const JAVASCRIPT: &str = r#"
(function_declaration name: (identifier) @name) @definition.function
(method_definition name: (property_identifier) @name) @definition.function
(variable_declarator name: (identifier) @name value: (arrow_function)) @definition.function
(class_declaration name: (identifier) @name) @definition.type
"#;

/// TypeScript and TSX share these. TSX differs only in its grammar.
pub const TYPESCRIPT: &str = r#"
(function_declaration name: (identifier) @name) @definition.function
(method_definition name: (property_identifier) @name) @definition.function
(abstract_method_signature name: (property_identifier) @name) @definition.function
(variable_declarator name: (identifier) @name value: (arrow_function)) @definition.function
(class_declaration name: (type_identifier) @name) @definition.type
(abstract_class_declaration name: (type_identifier) @name) @definition.type
(interface_declaration name: (type_identifier) @name) @definition.type
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.type
(internal_module name: (identifier) @name) @definition.module
"#;

pub const GO: &str = r#"
(function_declaration name: (identifier) @name) @definition.function
(method_declaration name: (field_identifier) @name) @definition.function
(type_spec name: (type_identifier) @name) @definition.type
(package_clause (package_identifier) @name) @definition.module
"#;
