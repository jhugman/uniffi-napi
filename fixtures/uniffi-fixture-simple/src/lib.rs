uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ArithmeticError {
    #[error("{reason}")]
    DivisionByZero { reason: String },
}

#[uniffi::export]
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[uniffi::export]
pub fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

#[uniffi::export]
pub fn divide(a: f64, b: f64) -> Result<f64, ArithmeticError> {
    if b == 0.0 {
        Err(ArithmeticError::DivisionByZero {
            reason: "cannot divide by zero".to_string(),
        })
    } else {
        Ok(a / b)
    }
}

#[uniffi::export]
pub async fn async_add(a: u32, b: u32) -> u32 {
    a + b
}

#[uniffi::export]
pub async fn async_greet(name: String) -> String {
    format!("Hello, {name}!")
}
