//! Shared validation for persisted preparation state and imported profiles.
use serde_json::Value;

fn day(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.parse::<chrono::NaiveDate>()
            .is_ok_and(|date| date.to_string() == s)
    })
}

pub fn validate(value: &Value) -> Result<(), String> {
    let fields = value
        .as_object()
        .ok_or("Invalid preparation state object")?;
    for (key, value) in fields {
        let valid = match key.as_str() {
            // Empty is the initial marker before the first service tick.
            "day" => value == "" || day(value),
            "attempts" => value.as_u64().is_some(),
            "next" => {
                value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|s| s.parse::<chrono::DateTime<chrono::Utc>>().is_ok())
            }
            "cancelled_day" => value.is_null() || day(value),
            "daily_complete" | "support_complete" => value.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid preparation state field: {key}"));
        }
    }
    Ok(())
}

pub fn validate_meta(key: &str, value: &Value) -> Result<(), String> {
    match key {
        "preparation_state" => validate(value),
        "legacy_preparation_states" => {
            for entry in value
                .as_array()
                .ok_or("Invalid legacy preparation states")?
            {
                validate(entry)?;
            }
            Ok(())
        }
        _ => Err("Unknown preparation metadata".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn accepts_old_and_current_states_but_rejects_invalid_dates_and_types() {
        for good in [
            json!({}),
            json!({"day":"","attempts":0,"next":null}),
            json!({"day":"2026-10-29","attempts":7,"next":"2026-10-29T12:00:00Z","cancelled_day":null}),
            json!({"day":"2026-10-29","daily_complete":true,"support_complete":false}),
        ] {
            validate(&good).unwrap();
        }
        for bad in [
            json!([]),
            json!({"next":"broken"}),
            json!({"day":"2026-02-30"}),
            json!({"attempts":-1}),
            json!({"cancelled_day":true}),
            json!({"daily_complete":"true"}),
            json!({"extra":"unknown"}),
        ] {
            assert!(validate(&bad).is_err(), "accepted {bad}");
        }
    }
}
