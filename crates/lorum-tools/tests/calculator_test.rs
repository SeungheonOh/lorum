use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry() -> ToolRegistry {
    let dir = std::env::temp_dir();
    ToolRegistry::new(dir, Duration::from_secs(30))
}

fn calc_call(id: &str, expression: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "calculator".to_string(),
        arguments: json!({ "expression": expression }),
    }
}

#[tokio::test]
async fn basic_addition() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t1", "2 + 3")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "5");
}

#[tokio::test]
async fn basic_multiplication() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t2", "10 * 5")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "50");
}

#[tokio::test]
async fn division() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t3", "20 / 4")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "5");
}

#[tokio::test]
async fn modulo() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t4", "17 % 5")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "2");
}

#[tokio::test]
async fn parentheses() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t5", "(2 + 3) * 4")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "20");
}

#[tokio::test]
async fn unary_minus() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t6", "-5 + 3")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "-2");
}

#[tokio::test]
async fn sqrt_function() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t7", "sqrt(16)")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "4");
}

#[tokio::test]
async fn pow_function() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t8", "pow(2, 8)")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "256");
}

#[tokio::test]
async fn constant_pi() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t9", "pi")).await;
    assert!(!result.is_error);
    let val: f64 = result.result.as_str().unwrap().parse().unwrap();
    assert!((val - std::f64::consts::PI).abs() < 1e-10);
}

#[tokio::test]
async fn constant_e() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t10", "e")).await;
    assert!(!result.is_error);
    let val: f64 = result.result.as_str().unwrap().parse().unwrap();
    assert!((val - std::f64::consts::E).abs() < 1e-10);
}

#[tokio::test]
async fn division_by_zero() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t11", "10 / 0")).await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("division by zero"));
}

#[tokio::test]
async fn invalid_expression() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t12", "2 + + +")).await;
    assert!(result.is_error);
}

#[tokio::test]
async fn nested_functions() {
    let registry = make_registry();
    let result = registry.execute(&calc_call("t13", "sqrt(abs(-16))")).await;
    assert!(!result.is_error);
    assert_eq!(result.result.as_str().unwrap(), "4");
}
