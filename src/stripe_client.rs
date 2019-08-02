use instrumented::instrument;

/// The list of possible values for a RequestError's type.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub enum ErrorType {
    #[serde(skip_deserializing)]
    Unknown,

    #[serde(rename = "api_error")]
    Api,
    #[serde(rename = "api_connection_error")]
    Connection,
    #[serde(rename = "authentication_error")]
    Authentication,
    #[serde(rename = "card_error")]
    Card,
    #[serde(rename = "invalid_request_error")]
    InvalidRequest,
    #[serde(rename = "rate_limit_error")]
    RateLimit,
    #[serde(rename = "validation_error")]
    Validation,
}

impl Default for ErrorType {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct RequestError {
    /// The HTTP status in the response.
    #[serde(skip_deserializing)]
    pub http_status: u16,

    /// The type of error returned.
    #[serde(rename = "type")]
    pub error_type: ErrorType,

    /// A human-readable message providing more details about the error.
    /// For card errors, these messages can be shown to end users.
    #[serde(default)]
    pub message: Option<String>,

    /// For card errors, a value describing the kind of card error that occured.
    pub code: Option<stripe::ErrorCode>,

    /// For card errors resulting from a bank decline, a string indicating the
    /// bank's reason for the decline if they provide one.
    pub decline_code: Option<String>,

    /// The ID of the failed charge, if applicable.
    pub charge: Option<String>,
}

#[derive(Debug, Fail)]
pub enum StripeError {
    #[fail(display = "stripe request error: {}", err)]
    RequestError {
        err: String,
        request_error: RequestError,
    },
    #[fail(display = "stripe error: {}", err)]
    Error { err: String },
    #[fail(display = "json parser error: {}", err)]
    JsonParserError { err: String },
}

impl From<serde_json::error::Error> for StripeError {
    fn from(err: serde_json::error::Error) -> Self {
        Self::JsonParserError {
            err: err.to_string(),
        }
    }
}

impl From<stripe::ErrorType> for ErrorType {
    fn from(et: stripe::ErrorType) -> Self {
        match et {
            stripe::ErrorType::Unknown => Self::Unknown,
            stripe::ErrorType::Api => Self::Api,
            stripe::ErrorType::Connection => Self::Connection,
            stripe::ErrorType::Authentication => Self::Authentication,
            stripe::ErrorType::Card => Self::Card,
            stripe::ErrorType::InvalidRequest => Self::InvalidRequest,
            stripe::ErrorType::RateLimit => Self::RateLimit,
            stripe::ErrorType::Validation => Self::Validation,
        }
    }
}

impl From<stripe::RequestError> for RequestError {
    fn from(re: stripe::RequestError) -> Self {
        Self {
            http_status: re.http_status,
            error_type: re.error_type.into(),
            message: re.message,
            code: re.code,
            decline_code: re.decline_code,
            charge: re.charge,
        }
    }
}

impl From<stripe::Error> for StripeError {
    fn from(err: stripe::Error) -> Self {
        let msg = err.to_string();
        match err {
            stripe::Error::Stripe(re) => Self::RequestError {
                err: msg,
                request_error: re.into(),
            },
            _ => Self::Error { err: msg },
        }
    }
}

pub struct Stripe {
    client: stripe::r#async::Client,
}

impl Stripe {
    pub fn new() -> Self {
        use dotenv::{dotenv, var};

        dotenv().ok();

        Self {
            client: stripe::r#async::Client::new(var("STRIPE_API_SECRET").unwrap()),
        }
    }

    pub fn calculate_stripe_fees(amount: i64) -> i64 {
        // Details on stripe fees: https://stripe.com/pricing#pricing-details
        ((amount as f64) * 0.029).floor() as i64 + 30
    }

    #[instrument(INFO)]
    pub fn charge(
        &self,
        token: &str,
        amount: i64,
        client_id: &str,
        tx_id: i64,
    ) -> Result<stripe::Charge, StripeError> {
        use crate::futures::Future;

        let token: stripe::Token = serde_json::from_str(token)?;
        let mut params = stripe::CreateCharge::new();

        params.amount = Some(amount);
        params.source = Some(stripe::ChargeSourceParams::Token(token.id));
        params.currency = Some(stripe::Currency::USD);
        params.capture = Some(true);

        let mut metadata = stripe::Metadata::new();
        metadata.insert("client_id".into(), client_id.into());
        metadata.insert("tx_id".into(), format!("{}", tx_id));
        params.metadata = Some(metadata);

        stripe::Charge::create(&self.client, params)
            .map_err(StripeError::from)
            .wait()
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_stripe_charge() {
        let stripe = Stripe::new();
        let token = r#"
        {
            "id": "tok_visa",
            "object": "token",
            "card": {
                "id": "card_1EYyYcG27b2IeIO74TusmAci",
                "object": "card",
                "address_city": null,
                "address_country": null,
                "address_line1": null,
                "address_line1_check": null,
                "address_line2": null,
                "address_state": null,
                "address_zip": null,
                "address_zip_check": null,
                "brand": "Visa",
                "country": "US",
                "cvc_check": null,
                "dynamic_last4": null,
                "exp_month": 8,
                "exp_year": 2020,
                "fingerprint": "9vruG6eJZVIM6012",
                "funding": "credit",
                "last4": "4242",
                "metadata": {},
                "name": null,
                "tokenization_method": null
            },
            "client_ip": null,
            "created": 1557594022,
            "livemode": false,
            "type": "card",
            "used": false
        }"#;
        stripe.charge(&token, 1000, "client_id", 100).unwrap();
    }

    #[test]
    fn test_stripe_fee_calculation() {
        for i in 0..10 {
            assert_eq!(Stripe::calculate_stripe_fees(1000 + i), 59);
            assert_eq!(Stripe::calculate_stripe_fees(10000 + i), 320);
        }
    }
}
