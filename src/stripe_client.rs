use instrumented::instrument;
use regex::Regex;

use crate::config;

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

#[derive(Debug, Deserialize, Serialize)]
pub struct ConnectCredentials {
    pub access_token: String,
    pub livemode: bool,
    pub refresh_token: String,
    pub token_type: String,
    pub stripe_publishable_key: String,
    pub stripe_user_id: String,
    pub scope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoginLink {
    pub object: String,
    pub created: i64,
    pub url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateLoginLink {
    pub account: String,
    pub redirect_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StripeUser {
    pub business_type: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateOauthUrl {
    pub client_id: String,
    pub state: String,
    pub redirect_uri: String,
    pub stripe_user: StripeUser,
    pub suggested_capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateTransfer {
    pub amount: i64,
    pub currency: stripe::Currency,
    pub destination: String,
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
    #[fail(display = "request error: {}", err)]
    RequestError {
        err: String,
        request_error: RequestError,
    },
    #[fail(display = "{}", err)]
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

impl From<reqwest::Error> for StripeError {
    fn from(err: reqwest::Error) -> Self {
        Self::Error {
            err: err.to_string(),
        }
    }
}

pub struct Stripe {
    client_secret: String,
    client: stripe::r#async::Client,
    connect_client_id: String,
    redirect_uri: String,
}

impl Stripe {
    pub fn new() -> Self {
        use dotenv::{dotenv, var};

        dotenv().ok();

        let client_secret = var("STRIPE_API_SECRET").expect("Missing Stripe API secret key");

        Self {
            client_secret: client_secret.clone(),
            client: stripe::r#async::Client::new(client_secret.clone()),
            connect_client_id: config::CONFIG.stripe.connect_client_id.clone(),
            redirect_uri: config::CONFIG.stripe.redirect_uri.clone(),
        }
    }

    pub fn calculate_stripe_fees(amount: i64) -> i64 {
        // Details on stripe fees: https://stripe.com/pricing#pricing-details
        ((amount as f64) * 0.029).round() as i64 + 30
    }

    pub fn get_oauth_url(&self, state: String) -> String {
        let qs = CreateOauthUrl {
            client_id: self.connect_client_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            state,
            stripe_user: StripeUser {
                business_type: "individual".to_string(),
            },
            suggested_capabilities: vec!["platform_payments".into()],
        };

        // This is a hack because Stripe does not accept lists with an index
        // value (i.e., [0] instead of []) in the query string.
        let re = Regex::new(r"\[\d+\]").unwrap();
        re.replace_all(
            &format!(
                "https://connect.stripe.com/express/oauth/authorize?{}",
                serde_qs::to_string(&qs).unwrap()
            ),
            "[]",
        )
        .into()
    }

    #[instrument(INFO)]
    pub fn post_connect_code(&self, code: &str) -> Result<ConnectCredentials, StripeError> {
        use futures::Future;
        use tokio::executor::Executor;
        let client = reqwest::r#async::Client::new();

        let params = [
            ("client_secret", self.client_secret.clone()),
            ("code", code.into()),
            ("grant_type", "authorization_code".into()),
        ];

        let mut exec = tokio::executor::DefaultExecutor::current();

        let (tx, rx) = futures::sync::oneshot::channel();
        exec.spawn(Box::new(
            client
                .post("https://connect.stripe.com/oauth/token")
                .form(&params)
                .send()
                .and_then(|mut resp| resp.text())
                .map(|credentials| credentials)
                .then(move |r| tx.send(r).map_err(|_werr| error!("failure"))),
        ))
        .unwrap();
        let credentials = rx.wait().unwrap()?;
        info!("creds: {}", credentials);
        let credentials: ConnectCredentials = serde_json::from_str(&credentials)?;

        Ok(credentials)
    }

    #[instrument(INFO)]
    pub fn get_login_link(&self, stripe_user_id: &str) -> Result<LoginLink, StripeError> {
        use futures::Future;
        use tokio::executor::Executor;

        let path = format!("/accounts/{}/login_links", stripe_user_id);

        let mut exec = tokio::executor::DefaultExecutor::current();

        let (tx, rx) = futures::sync::oneshot::channel();
        exec.spawn(Box::new(
            self.client
                .post_form::<LoginLink, CreateLoginLink>(
                    &path,
                    CreateLoginLink {
                        account: stripe_user_id.into(),
                        redirect_url: self.redirect_uri.clone(),
                    },
                )
                .then(move |r| tx.send(r))
                .map_err(|err| error!("failure: {:?}", err)),
        ))
        .unwrap();
        rx.wait().unwrap().map_err(StripeError::from)
    }

    #[instrument(INFO)]
    pub fn charge(
        &self,
        token: &str,
        amount: i64,
        client_id: &str,
        tx_id: i64,
    ) -> Result<stripe::Charge, StripeError> {
        use futures::Future;
        use tokio::executor::Executor;

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

        let mut exec = tokio::executor::DefaultExecutor::current();

        let (tx, rx) = futures::sync::oneshot::channel();
        exec.spawn(Box::new(
            stripe::Charge::create(&self.client, params)
                .then(move |r| tx.send(r))
                .map_err(|err| error!("failure: {:?}", err)),
        ))
        .unwrap();
        rx.wait().unwrap().map_err(StripeError::from)
    }

    #[instrument(INFO)]
    pub fn transfer(
        &self,
        amount: i32,
        stripe_user_id: &str,
    ) -> Result<stripe::Transfer, StripeError> {
        use futures::Future;
        use tokio::executor::Executor;

        let transfer = CreateTransfer {
            amount: i64::from(amount),
            destination: stripe_user_id.into(),
            currency: stripe::Currency::USD,
        };

        let mut exec = tokio::executor::DefaultExecutor::current();

        let (tx, rx) = futures::sync::oneshot::channel();
        exec.spawn(Box::new(
            self.client
                .post_form::<stripe::Transfer, CreateTransfer>("/transfer", transfer)
                .then(move |r| tx.send(r))
                .map_err(|err| error!("failure: {:?}", err)),
        ))
        .unwrap();
        rx.wait().unwrap().map_err(StripeError::from)
    }

    #[instrument(INFO)]
    pub fn get_account(&self, stripe_user_id: &str) -> Result<stripe::Account, StripeError> {
        use futures::Future;
        use std::str::FromStr;
        use tokio::executor::Executor;

        let mut exec = tokio::executor::DefaultExecutor::current();

        let (tx, rx) = futures::sync::oneshot::channel();
        exec.spawn(Box::new(
            stripe::Account::retrieve(
                &self.client,
                &stripe::AccountId::from_str(stripe_user_id).unwrap(),
                &[],
            )
            .then(move |r| tx.send(r))
            .map_err(|err| error!("failure: {:?}", err)),
        ))
        .unwrap();
        rx.wait().unwrap().map_err(StripeError::from)
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;
    use futures::future;

    #[test]
    fn test_stripe_charge() {
        tokio::run(future::lazy(|| {
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

            future::ok(())
        }));
    }

    #[test]
    fn test_stripe_fee_calculation() {
        for i in 0..10 {
            assert_eq!(Stripe::calculate_stripe_fees(1000 + i), 59);
            assert_eq!(Stripe::calculate_stripe_fees(10000 + i), 320);
        }
        assert_eq!(Stripe::calculate_stripe_fees(2091), 91);
    }

    #[test]
    fn test_get_oauth_url() {
        let stripe = Stripe::new();
        let url = stripe.get_oauth_url("somestate".to_string());
        assert_eq!(
            url,
            "https://connect.stripe.com/express/oauth/authorize?\
             client_id=ca_FVZ7xsdnQsZChPyqzq4sDtwCMSoATpPz\
             &state=somestate\
             &redirect_uri=https%3A%2F%2Fstaging.umpyre.io%2Faccounts%2Fpayouts\
             &stripe_user[business_type]=individual\
             &suggested_capabilities[]=platform_payments"
        )
    }
}
