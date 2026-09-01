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

pub fn positive_dimension(name: &str, value: &str) -> io::Result<f64> {
    let number = finite_number(name, value)?;
    if number > 0.0 {
        Ok(number)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be greater than 0"),
        ))
    }
}

pub fn rotation(value: &str) -> io::Result<f64> {
    let number = finite_number("Rotation", value)?;
    if (-360.0..=360.0).contains(&number) {
        Ok(number)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Rotation must be between -360 and 360 degrees",
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

    #[test]
    fn positive_dimension_rejects_zero_and_negative() {
        assert!(positive_dimension("Width", "0").is_err());
        assert!(positive_dimension("Width", "-1").is_err());
        assert_eq!(positive_dimension("Width", "152.4").unwrap(), 152.4);
    }

    #[test]
    fn rotation_rejects_out_of_range_values() {
        assert!(rotation("361").is_err());
        assert!(rotation("-361").is_err());
        assert_eq!(rotation("-360").unwrap(), -360.0);
        assert_eq!(rotation("360").unwrap(), 360.0);
    }
}
