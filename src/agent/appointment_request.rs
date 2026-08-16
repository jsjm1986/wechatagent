//! Deterministic validation for Agent-produced appointment request envelopes.
//!
//! The model owns whether the customer is asking for a visit. This module owns only the
//! structured boundary required before that semantic decision can become durable state.

use mongodb::bson::DateTime;

use super::types::AppointmentRequestDecision;

pub(crate) const MAX_APPOINTMENT_REQUEST_TEXT_CHARS: usize = 2_000;
pub(crate) const MAX_APPOINTMENT_LOCATION_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedAppointmentRequest {
    pub request_text: String,
    pub preferred_start: Option<DateTime>,
    pub preferred_end: Option<DateTime>,
    pub location_preference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppointmentRequestValidationError {
    MissingRequestText,
    RequestTextTooLong,
    InvalidPreferredStart,
    InvalidPreferredEnd,
    InvalidPreferredWindow,
    LocationTooLong,
}

impl AppointmentRequestValidationError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::MissingRequestText => "request_text_missing",
            Self::RequestTextTooLong => "request_text_too_long",
            Self::InvalidPreferredStart => "preferred_start_invalid_rfc3339",
            Self::InvalidPreferredEnd => "preferred_end_invalid_rfc3339",
            Self::InvalidPreferredWindow => "preferred_end_not_after_start",
            Self::LocationTooLong => "location_preference_too_long",
        }
    }

    pub(crate) fn repair_instruction(self) -> &'static str {
        match self {
            Self::MissingRequestText => {
                "The appointment request is missing requestText. Preserve only the customer's actual request in a concise requestText, or remove appointmentRequest when no supported request exists."
            }
            Self::RequestTextTooLong => {
                "Shorten appointmentRequest.requestText to a concise customer-request summary without adding facts, or remove appointmentRequest when no supported request exists."
            }
            Self::InvalidPreferredStart => {
                "appointmentRequest.preferredStart must be an RFC3339 timestamp or an empty string. Repair only from supported customer context; otherwise use an empty string."
            }
            Self::InvalidPreferredEnd => {
                "appointmentRequest.preferredEnd must be an RFC3339 timestamp or an empty string. Repair only from supported customer context; otherwise use an empty string."
            }
            Self::InvalidPreferredWindow => {
                "appointmentRequest.preferredEnd must be later than preferredStart. Correct the structured interval only when supported by the customer context; otherwise clear the uncertain timestamp."
            }
            Self::LocationTooLong => {
                "Shorten appointmentRequest.locationPreference without adding facts, or clear it when the customer's location preference is not supported."
            }
        }
    }
}

pub(crate) fn validate_appointment_request(
    request: Option<&AppointmentRequestDecision>,
) -> Result<Option<ValidatedAppointmentRequest>, AppointmentRequestValidationError> {
    let Some(request) = request.filter(|request| request.requested) else {
        return Ok(None);
    };

    let request_text = request.request_text.trim();
    if request_text.is_empty() {
        return Err(AppointmentRequestValidationError::MissingRequestText);
    }
    if request_text.chars().count() > MAX_APPOINTMENT_REQUEST_TEXT_CHARS {
        return Err(AppointmentRequestValidationError::RequestTextTooLong);
    }

    let preferred_start = parse_optional_datetime(
        &request.preferred_start,
        AppointmentRequestValidationError::InvalidPreferredStart,
    )?;
    let preferred_end = parse_optional_datetime(
        &request.preferred_end,
        AppointmentRequestValidationError::InvalidPreferredEnd,
    )?;
    if let (Some(start), Some(end)) = (preferred_start, preferred_end) {
        if end.timestamp_millis() <= start.timestamp_millis() {
            return Err(AppointmentRequestValidationError::InvalidPreferredWindow);
        }
    }

    let location_preference = request.location_preference.trim();
    if location_preference.chars().count() > MAX_APPOINTMENT_LOCATION_CHARS {
        return Err(AppointmentRequestValidationError::LocationTooLong);
    }

    Ok(Some(ValidatedAppointmentRequest {
        request_text: request_text.to_string(),
        preferred_start,
        preferred_end,
        location_preference: (!location_preference.is_empty())
            .then(|| location_preference.to_string()),
    }))
}

fn parse_optional_datetime(
    value: &str,
    error: AppointmentRequestValidationError,
) -> Result<Option<DateTime>, AppointmentRequestValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    DateTime::parse_rfc3339_str(value)
        .map(Some)
        .map_err(|_| error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AppointmentRequestDecision {
        AppointmentRequestDecision {
            requested: true,
            request_text: "客户希望到院面诊".to_string(),
            preferred_start: "2026-08-20T10:00:00+08:00".to_string(),
            preferred_end: "2026-08-20T11:00:00+08:00".to_string(),
            location_preference: "院区待确认".to_string(),
            reason: "记录客户请求".to_string(),
        }
    }

    #[test]
    fn validates_and_normalizes_structured_request() {
        let mut request = request();
        request.request_text = "  客户希望到院面诊  ".to_string();
        let validated = validate_appointment_request(Some(&request))
            .unwrap()
            .expect("active request");
        assert_eq!(validated.request_text, "客户希望到院面诊");
        assert!(validated.preferred_start.is_some());
        assert!(validated.preferred_end.is_some());
        assert_eq!(validated.location_preference.as_deref(), Some("院区待确认"));
    }

    #[test]
    fn rejects_invalid_or_reversed_time_windows() {
        let mut invalid = request();
        invalid.preferred_start = "tomorrow morning".to_string();
        assert_eq!(
            validate_appointment_request(Some(&invalid)),
            Err(AppointmentRequestValidationError::InvalidPreferredStart)
        );

        let mut reversed = request();
        reversed.preferred_end = "2026-08-20T09:00:00+08:00".to_string();
        assert_eq!(
            validate_appointment_request(Some(&reversed)),
            Err(AppointmentRequestValidationError::InvalidPreferredWindow)
        );
    }
}
