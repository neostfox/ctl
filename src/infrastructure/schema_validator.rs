use anyhow::{anyhow, Result};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct SchemaValidator {
    schemas: Vec<Value>,
}

impl SchemaValidator {
    pub fn new(schema_dir: &str) -> Result<Self> {
        let mut schemas = Vec::new();
        if Path::new(schema_dir).exists() {
            for entry in fs::read_dir(schema_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    let content = fs::read_to_string(&path)?;
                    let schema: Value = serde_json::from_str(&content)?;
                    schemas.push(schema);
                }
            }
        }
        Ok(Self { schemas })
    }

    pub fn validate_instance(&self, instance: &Value, schema_id: &str) -> Result<()> {
        let schema_value = self.schemas.iter().find(|s| {
            s.get("$id")
                .and_then(|v| v.as_str())
                .map(|id| id == schema_id || id.starts_with(schema_id) || id.contains(schema_id))
                .unwrap_or(false)
        });

        if let Some(schema) = schema_value {
            self.validate_node(instance, schema, "root")?;
            Ok(())
        } else {
            Err(anyhow!("Schema not found: {}", schema_id))
        }
    }

    fn validate_node(&self, instance: &Value, schema: &Value, path: &str) -> Result<()> {
        if let Some(typ) = schema.get("type") {
            let type_str = typ.as_str().unwrap_or("");
            let valid = match type_str {
                "object" => instance.is_object(),
                "array" => instance.is_array(),
                "string" => instance.is_string(),
                "integer" => instance.is_i64() || instance.is_u64(),
                "boolean" => instance.is_boolean(),
                "number" => instance.is_number(),
                _ => true,
            };
            if !valid {
                return Err(anyhow!("Type mismatch at {}: expected {}", path, type_str));
            }
        }

        if let Some(const_val) = schema.get("const") {
            if instance != const_val {
                return Err(anyhow!(
                    "Const mismatch at {}: expected {}",
                    path,
                    const_val
                ));
            }
        }

        if let Some(enums) = schema.get("enum") {
            if let Some(arr) = enums.as_array() {
                if !arr.contains(instance) {
                    return Err(anyhow!("Value at {} not in enum", path));
                }
            }
        }

        if let Some(format) = schema.get("format").and_then(|v| v.as_str()) {
            if let Some(s) = instance.as_str() {
                match format {
                    "uuid" => {
                        let parts: Vec<&str> = s.split('-').collect();
                        if parts.len() != 5
                            || parts[0].len() != 8
                            || parts[1].len() != 4
                            || parts[2].len() != 4
                            || parts[3].len() != 4
                            || parts[4].len() != 12
                            || !s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                        {
                            return Err(anyhow!("Invalid UUID at {}: {}", path, s));
                        }
                    }
                    "date-time" if !is_valid_iso8601_datetime(s) => {
                        return Err(anyhow!("Invalid date-time at {}: {}", path, s));
                    }
                    _ => {}
                }
            }
        }

        if let (Some(min), Some(val)) = (schema.get("minimum"), instance.as_i64()) {
            if let Some(min_val) = min.as_i64() {
                if val < min_val {
                    return Err(anyhow!(
                        "Value {} at {} is less than minimum {}",
                        val,
                        path,
                        min_val
                    ));
                }
            }
        }
        // minLength
        if let (Some(min_len), Some(s)) = (schema.get("minLength"), instance.as_str()) {
            if let Some(min) = min_len.as_u64() {
                if s.len() < min as usize {
                    return Err(anyhow!(
                        "String at {} is too short: {} < {}",
                        path,
                        s.len(),
                        min
                    ));
                }
            }
        }

        if let (Some(min_items), Some(arr)) = (schema.get("minItems"), instance.as_array()) {
            if let Some(min) = min_items.as_u64() {
                if arr.len() < min as usize {
                    return Err(anyhow!(
                        "Array at {} has too few items: {} < {}",
                        path,
                        arr.len(),
                        min
                    ));
                }
            }
        }

        if let (Some(req), Some(obj)) = (schema.get("required"), instance.as_object()) {
            if let Some(req_arr) = req.as_array() {
                for field in req_arr {
                    if let Some(field_name) = field.as_str() {
                        if !obj.contains_key(field_name) {
                            return Err(anyhow!(
                                "Missing required field at {}: {}",
                                path,
                                field_name
                            ));
                        }
                    }
                }
            }
        }

        if let (Some(obj), Some(props), Some(false)) = (
            instance.as_object(),
            schema.get("properties"),
            schema
                .get("unevaluatedProperties")
                .and_then(|v| v.as_bool()),
        ) {
            if let Some(props_obj) = props.as_object() {
                for key in obj.keys() {
                    if !props_obj.contains_key(key) {
                        return Err(anyhow!("Unevaluated property at {}: {}", path, key));
                    }
                }
            }
        }
        // additionalProperties: false — reject undeclared properties.
        // When a schema omits `properties`, the allowed set is empty.
        if let (Some(obj), Some(false)) = (
            instance.as_object(),
            schema.get("additionalProperties").and_then(|v| v.as_bool()),
        ) {
            let props_obj = schema.get("properties").and_then(|v| v.as_object());
            for key in obj.keys() {
                if props_obj.is_none_or(|props| !props.contains_key(key)) {
                    return Err(anyhow!("Additional property at {}: {}", path, key));
                }
            }
        }

        // allOf — validate against every sub-schema
        if let Some(all_of) = schema.get("allOf").and_then(|v| v.as_array()) {
            for (i, sub_schema) in all_of.iter().enumerate() {
                self.validate_node(instance, sub_schema, &format!("{}(allOf[{}])", path, i))?;
            }
        }

        // if/then — conditional validation
        if let Some(if_schema) = schema.get("if") {
            if self
                .validate_node(instance, if_schema, &format!("{}(if)", path))
                .is_ok()
            {
                if let Some(then_schema) = schema.get("then") {
                    self.validate_node(instance, then_schema, &format!("{}(then)", path))?;
                }
            }
        }

        if let (Some(arr), Some(items_schema)) = (instance.as_array(), schema.get("items")) {
            for (i, item) in arr.iter().enumerate() {
                self.validate_node(item, items_schema, &format!("{}[{}]", path, i))?;
            }
        }

        if let (Some(obj), Some(props)) = (instance.as_object(), schema.get("properties")) {
            if let Some(props_obj) = props.as_object() {
                for (key, prop_schema) in props_obj {
                    if let Some(val) = obj.get(key) {
                        self.validate_node(val, prop_schema, &format!("{}.{}", path, key))?;
                    }
                }
            }
        }

        Ok(())
    }
}

fn is_valid_iso8601_datetime(s: &str) -> bool {
    let bytes = s.as_bytes();
    let len = bytes.len();
    // Minimum: YYYY-MM-DDTHH:MM:SSZ (20 chars)
    if len < 20 {
        return false;
    }
    // YYYY-MM-DDTHH:MM:SS (first 19 chars fixed)
    let d = |i: usize| bytes.get(i).map(|b| b.is_ascii_digit()).unwrap_or(false);
    if !(d(0)
        && d(1)
        && d(2)
        && d(3)
        && bytes[4] == b'-'
        && d(5)
        && d(6)
        && bytes[7] == b'-'
        && d(8)
        && d(9)
        && bytes[10] == b'T'
        && d(11)
        && d(12)
        && bytes[13] == b':'
        && d(14)
        && d(15)
        && bytes[16] == b':'
        && d(17)
        && d(18))
    {
        return false;
    }
    // Timezone suffix: Z or ±HH:MM (with optional fractional seconds)
    let rest = &s[19..];
    if rest == "Z" {
        return true;
    }
    let tz_part = if let Some(stripped) = rest.strip_prefix('.') {
        // Find end of fractional seconds
        let digits_end = stripped
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| i + 1)
            .unwrap_or(stripped.len());
        if digits_end < 1 {
            return false; // Must have at least one fractional digit
        }
        &rest[digits_end + 1..]
    } else {
        rest
    };
    // ±HH:MM
    let tz_bytes = tz_part.as_bytes();
    if tz_bytes.len() != 6 {
        return false;
    }
    (tz_bytes[0] == b'+' || tz_bytes[0] == b'-')
        && d_offset(tz_bytes, 1)
        && d_offset(tz_bytes, 2)
        && tz_bytes[3] == b':'
        && d_offset(tz_bytes, 4)
        && d_offset(tz_bytes, 5)
}

fn d_offset(bytes: &[u8], i: usize) -> bool {
    bytes.get(i).map(|b| b.is_ascii_digit()).unwrap_or(false)
}
