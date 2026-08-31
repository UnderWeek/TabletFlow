use std::io;

pub fn finite_number(name: &str, value: &str) -> io::Result<f64> {
    let number = value.parse::<f64>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid value for {name}"),
        )
    })?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid value for {name}"),
        ))
    }
}

pub fn frequency(value: &str) -> io::Result<f64> {
    let frequency = finite_number("Frequency", value)?;
    if (1.0..=1000.0).contains(&frequency) {
        Ok(frequency)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Frequency must be between 1 and 1000 Hz",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_rejects_invalid_values() {
        assert!(frequency("0").is_err());
        assert!(frequency("1001").is_err());
        assert!(frequency("nan").is_err());
        assert_eq!(frequency("500").unwrap(), 500.0);
    }
}
