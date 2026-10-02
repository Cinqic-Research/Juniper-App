use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const MAX_EXPRESSION_BYTES: usize = 256;
pub const MAX_TOOL_ROUNDS: u32 = 4;
pub const MAX_TOOL_CALLS_PER_ROUND: u32 = 8;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const PROTOCOL_VERSION: &str = "juniper-tool-protocol-v1";

#[derive(Debug, Error, PartialEq)]
pub enum ToolError {
    #[error("expression is too long")]
    TooLong,
    #[error("invalid expression")]
    Invalid,
    #[error("division by zero")]
    DivisionByZero,
    #[error("unsupported unit conversion")]
    UnsupportedUnit,
    #[error("invalid tool arguments")]
    InvalidArguments,
    #[error("unknown tool")]
    UnknownTool,
    #[error("calculator expression is too deeply nested")]
    TooDeep,
    #[error("calculator exponent is outside the supported range")]
    ExponentTooLarge,
    #[error("tool result exceeds the runtime limit")]
    ResultTooLarge,
}

pub fn evaluate(expression: &str) -> Result<f64, ToolError> {
    if expression.len() > MAX_EXPRESSION_BYTES {
        return Err(ToolError::TooLong);
    }
    let mut parser = Parser {
        input: expression.as_bytes(),
        position: 0,
        depth: 0,
        steps: 0,
    };
    let value = parser.expression()?;
    parser.skip_space();
    if parser.position != parser.input.len() || !value.is_finite() {
        return Err(ToolError::Invalid);
    }
    Ok(value)
}

pub fn convert(value: f64, from: &str, to: &str) -> Result<f64, ToolError> {
    let from = from.trim().to_ascii_lowercase();
    let to = to.trim().to_ascii_lowercase();
    let from_dimension = unit_dimension(&from).ok_or(ToolError::UnsupportedUnit)?;
    let to_dimension = unit_dimension(&to).ok_or(ToolError::UnsupportedUnit)?;
    if from_dimension != to_dimension {
        return Err(ToolError::UnsupportedUnit);
    }
    let base = match from_dimension {
        "length" => length_to_base(value, &from)?,
        "temperature" => temperature_to_base(value, &from)?,
        "mass" => mass_to_base(value, &from)?,
        "speed" => speed_to_base(value, &from)?,
        "data" => data_to_base(value, &from)?,
        _ => return Err(ToolError::UnsupportedUnit),
    };
    let result = match from_dimension {
        "length" => base_to_length(base, &to)?,
        "temperature" => base_to_temperature(base, &to)?,
        "mass" => base_to_mass(base, &to)?,
        "speed" => base_to_speed(base, &to)?,
        "data" => base_to_data(base, &to)?,
        _ => return Err(ToolError::UnsupportedUnit),
    };
    if !result.is_finite() {
        return Err(ToolError::Invalid);
    }
    Ok(result)
}

fn unit_dimension(unit: &str) -> Option<&'static str> {
    match unit {
        "m" | "meter" | "meters" | "km" | "kilometer" | "kilometers" | "cm" | "centimeter"
        | "centimeters" | "ft" | "foot" | "feet" | "mi" | "mile" | "miles" => Some("length"),
        "c" | "celsius" | "f" | "fahrenheit" | "k" | "kelvin" => Some("temperature"),
        "g" | "gram" | "grams" | "kg" | "kilogram" | "kilograms" | "lb" | "pound" | "pounds" => {
            Some("mass")
        }
        "m/s" | "km/h" | "mph" => Some("speed"),
        "b" | "byte" | "bytes" | "kb" | "kib" | "mb" | "mib" | "gb" | "gib" => Some("data"),
        _ => None,
    }
}

fn length_to_base(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "m" | "meter" | "meters" => value,
        "km" | "kilometer" | "kilometers" => value * 1000.0,
        "cm" | "centimeter" | "centimeters" => value / 100.0,
        "ft" | "foot" | "feet" => value * 0.3048,
        "mi" | "mile" | "miles" => value * 1609.344,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn base_to_length(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "m" | "meter" | "meters" => value,
        "km" | "kilometer" | "kilometers" => value / 1000.0,
        "cm" | "centimeter" | "centimeters" => value * 100.0,
        "ft" | "foot" | "feet" => value / 0.3048,
        "mi" | "mile" | "miles" => value / 1609.344,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn temperature_to_base(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "c" | "celsius" => value,
        "f" | "fahrenheit" => (value - 32.0) * 5.0 / 9.0,
        "k" | "kelvin" => value - 273.15,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn base_to_temperature(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "c" | "celsius" => value,
        "f" | "fahrenheit" => value * 9.0 / 5.0 + 32.0,
        "k" | "kelvin" => value + 273.15,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn mass_to_base(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "g" | "gram" | "grams" => value,
        "kg" | "kilogram" | "kilograms" => value * 1000.0,
        "lb" | "pound" | "pounds" => value * 453.59237,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn base_to_mass(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "g" | "gram" | "grams" => value,
        "kg" | "kilogram" | "kilograms" => value / 1000.0,
        "lb" | "pound" | "pounds" => value / 453.59237,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn speed_to_base(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "m/s" => value,
        "km/h" => value / 3.6,
        "mph" => value * 0.44704,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn base_to_speed(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "m/s" => value,
        "km/h" => value * 3.6,
        "mph" => value / 0.44704,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn data_to_base(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "b" | "byte" | "bytes" => value,
        "kb" => value * 1000.0,
        "kib" => value * 1024.0,
        "mb" => value * 1_000_000.0,
        "mib" => value * 1_048_576.0,
        "gb" => value * 1_000_000_000.0,
        "gib" => value * 1_073_741_824.0,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}
fn base_to_data(value: f64, unit: &str) -> Result<f64, ToolError> {
    Ok(match unit {
        "b" | "byte" | "bytes" => value,
        "kb" => value / 1000.0,
        "kib" => value / 1024.0,
        "mb" => value / 1_000_000.0,
        "mib" => value / 1_048_576.0,
        "gb" => value / 1_000_000_000.0,
        "gib" => value / 1_073_741_824.0,
        _ => return Err(ToolError::UnsupportedUnit),
    })
}

pub fn validate_call(name: &str, arguments: &Value) -> Result<(), ToolError> {
    if !matches!(
        name,
        "calculator.evaluate"
            | "datetime.current"
            | "unit.convert"
            | "memory.list"
            | "memory.save"
            | "memory.delete"
            | "chat.search"
            | "file.read"
            | "file.metadata"
            | "system.info"
    ) {
        return Err(ToolError::UnknownTool);
    }
    let object = arguments.as_object().ok_or(ToolError::InvalidArguments)?;
    let valid = match name {
        "calculator.evaluate" => {
            object.len() == 1
                && object
                    .get("expression")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.len() <= MAX_EXPRESSION_BYTES)
        }
        "datetime.current" | "memory.list" | "system.info" => object.is_empty(),
        "unit.convert" => {
            object.len() == 3
                && object.get("value").and_then(Value::as_f64).is_some()
                && object
                    .get("from")
                    .and_then(Value::as_str)
                    .is_some_and(valid_argument_string)
                && object
                    .get("to")
                    .and_then(Value::as_str)
                    .is_some_and(valid_argument_string)
        }
        "memory.save" => {
            object.len() == 1
                && object
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty() && value.len() <= 1000)
        }
        "memory.delete" => {
            object.len() == 1
                && object
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(valid_identifier)
        }
        "file.read" | "file.metadata" => {
            object.len() == 1
                && object
                    .get("attachmentId")
                    .and_then(Value::as_str)
                    .is_some_and(valid_identifier)
        }
        "chat.search" => {
            object.len() == 1
                && object
                    .get("query")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty() && value.len() <= 200)
        }
        _ => false,
    };
    if !valid {
        return Err(ToolError::InvalidArguments);
    }
    if serde_json::to_vec(arguments)
        .map_err(|_| ToolError::Invalid)?
        .len()
        > MAX_PAYLOAD_BYTES
    {
        return Err(ToolError::ResultTooLarge);
    }
    Ok(())
}

fn valid_argument_string(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub fn host_result(
    call_id: &str,
    name: &str,
    status: &str,
    result: Option<Value>,
    error: Option<Value>,
) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "callId": call_id,
        "name": name,
        "status": status,
        "result": result,
        "error": error
    })
}

pub fn execute_call(
    call_id: &str,
    name: &str,
    arguments: &Value,
    round: u32,
    calls_this_round: u32,
) -> Value {
    if !loop_allowed(round, calls_this_round) {
        return host_result(
            call_id,
            name,
            "denied",
            None,
            Some(json!({
                "code": "TOOL_LOOP_LIMIT",
                "message": "Tool loop limit reached."
            })),
        );
    }
    if let Err(error) = validate_call(name, arguments) {
        let code = if error == ToolError::UnknownTool {
            "UNKNOWN_TOOL"
        } else {
            "INVALID_TOOL_ARGUMENT"
        };
        return host_result(
            call_id,
            name,
            "error",
            None,
            Some(json!({ "code": code, "message": error.to_string() })),
        );
    }

    let result = match name {
        "calculator.evaluate" => arguments
            .get("expression")
            .and_then(Value::as_str)
            .ok_or(ToolError::Invalid)
            .and_then(evaluate)
            .map(|value| json!({ "value": value })),
        "unit.convert" => arguments
            .get("value")
            .and_then(Value::as_f64)
            .zip(arguments.get("from").and_then(Value::as_str))
            .zip(arguments.get("to").and_then(Value::as_str))
            .map(|((value, from), to)| {
                convert(value, from, to)
                    .map(|result| json!({ "value": result, "from": from, "to": to }))
            })
            .unwrap_or(Err(ToolError::Invalid)),
        "datetime.current" => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| json!({ "unixSeconds": duration.as_secs() }))
            .map_err(|_| ToolError::Invalid),
        _ => Err(ToolError::Invalid),
    };

    match result {
        Ok(value) => {
            let payload = serde_json::to_vec(&value).unwrap_or_default();
            if payload.len() > MAX_PAYLOAD_BYTES {
                host_result(
                    call_id,
                    name,
                    "error",
                    None,
                    Some(
                        json!({ "code": "TOOL_RESULT_TOO_LARGE", "message": "The host tool result exceeded the runtime limit." }),
                    ),
                )
            } else {
                host_result(call_id, name, "success", Some(value), None)
            }
        }
        Err(error) => host_result(
            call_id,
            name,
            "error",
            None,
            Some(json!({
                "code": "TOOL_EXECUTION_ERROR",
                "message": error.to_string()
            })),
        ),
    }
}

pub fn loop_allowed(round: u32, calls_this_round: u32) -> bool {
    round < MAX_TOOL_ROUNDS && calls_this_round <= MAX_TOOL_CALLS_PER_ROUND
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
    depth: usize,
    steps: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn expression(&mut self) -> Result<f64, ToolError> {
        self.step()?;
        let mut value = self.term()?;
        loop {
            self.skip_space();
            match self.input.get(self.position) {
                Some(b'+') => {
                    self.position += 1;
                    value += self.term()?;
                }
                Some(b'-') => {
                    self.position += 1;
                    value -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<f64, ToolError> {
        self.step()?;
        let mut value = self.power()?;
        loop {
            self.skip_space();
            match self.input.get(self.position) {
                Some(b'*') => {
                    self.position += 1;
                    value *= self.power()?;
                }
                Some(b'/') => {
                    self.position += 1;
                    let divisor = self.power()?;
                    if divisor == 0.0 {
                        return Err(ToolError::DivisionByZero);
                    }
                    value /= divisor;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn power(&mut self) -> Result<f64, ToolError> {
        self.step()?;
        let mut value = self.unary()?;
        self.skip_space();
        if self.input.get(self.position) == Some(&b'^') {
            self.position += 1;
            let exponent = self.power()?;
            if !exponent.is_finite() || exponent.abs() > 1000.0 {
                return Err(ToolError::ExponentTooLarge);
            }
            value = value.powf(exponent);
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<f64, ToolError> {
        self.skip_space();
        if self.input.get(self.position) == Some(&b'-') {
            self.position += 1;
            return Ok(-self.unary()?);
        }
        if self.input.get(self.position) == Some(&b'+') {
            self.position += 1;
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<f64, ToolError> {
        self.step()?;
        self.skip_space();
        if self.input.get(self.position) == Some(&b'(') {
            self.depth += 1;
            if self.depth > 32 {
                return Err(ToolError::TooDeep);
            }
            self.position += 1;
            let value = self.expression()?;
            self.skip_space();
            if self.input.get(self.position) != Some(&b')') {
                return Err(ToolError::Invalid);
            }
            self.position += 1;
            self.depth -= 1;
            return Ok(value);
        }
        let start = self.position;
        while self
            .input
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            self.position += 1;
        }
        if start == self.position {
            return Err(ToolError::Invalid);
        }
        std::str::from_utf8(&self.input[start..self.position])
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or(ToolError::Invalid)
    }

    fn step(&mut self) -> Result<(), ToolError> {
        self.steps += 1;
        if self.steps > 512 {
            return Err(ToolError::TooLong);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_is_deterministic() {
        assert_eq!(evaluate("847291 * 19347").unwrap(), 16392538977.0);
    }

    #[test]
    fn calculator_rejects_code() {
        assert_eq!(
            evaluate("1; process.exit()").unwrap_err(),
            ToolError::Invalid
        );
    }

    #[test]
    fn calculator_bounds_input() {
        assert_eq!(evaluate(&"1".repeat(257)).unwrap_err(), ToolError::TooLong);
    }

    #[test]
    fn conversion_is_explicit() {
        assert!((convert(1.0, "km", "m").unwrap() - 1000.0).abs() < f64::EPSILON);
        assert_eq!(
            convert(1.0, "parsec", "m").unwrap_err(),
            ToolError::UnsupportedUnit
        );
        assert_eq!(
            convert(5.0, "m", "celsius").unwrap_err(),
            ToolError::UnsupportedUnit
        );
    }

    #[test]
    fn tool_result_is_host_shaped() {
        let result = host_result(
            "x",
            "calculator.evaluate",
            "success",
            Some(json!({ "value": 2 })),
            None,
        );
        assert_eq!(result["protocolVersion"], "juniper-tool-protocol-v1");
        assert_eq!(result["status"], "success");
    }

    #[test]
    fn host_executes_only_approved_safe_calls() {
        let result = execute_call(
            "x",
            "calculator.evaluate",
            &json!({ "expression": "2 + 2" }),
            0,
            1,
        );
        assert_eq!(result["status"], "success");
        assert_eq!(result["result"]["value"], 4.0);

        let unsupported = execute_call("x", "memory.save", &json!({ "content": "secret" }), 0, 1);
        assert_eq!(unsupported["status"], "error");
    }

    #[test]
    fn loop_is_bounded() {
        assert!(loop_allowed(0, 8));
        assert!(!loop_allowed(4, 0));
        assert!(!loop_allowed(0, 9));
    }

    #[test]
    fn malformed_and_unknown_arguments_are_rejected() {
        assert_eq!(
            validate_call("calculator.evaluate", &json!({})).unwrap_err(),
            ToolError::InvalidArguments
        );
        assert_eq!(
            validate_call(
                "calculator.evaluate",
                &json!({ "expression": "2+2", "extra": true })
            )
            .unwrap_err(),
            ToolError::InvalidArguments
        );
        assert_eq!(
            validate_call("unknown.tool", &json!({})).unwrap_err(),
            ToolError::UnknownTool
        );
    }
}
