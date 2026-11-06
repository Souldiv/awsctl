#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    NeedsLogin,
    Transient,
}

const LOGIN_MARKERS: &[&str] = &[
    "expiredtoken",
    "unrecognizedclient",
    "invalidclienttokenid",
    "invalidgrant",
    "unauthorizedexception",
    "sso token has expired",
    "token has expired",
    "security token included in the request is",
    "generate credentials and try again",
];

// The SDK reports a credentials failure this way whatever the cause, and the
// cause is often only the network being down, after a laptop wakes up.
const CREDENTIAL_LOADING_MARKERS: &[&str] = &[
    "credential provider was not enabled",
    "credentials provider was not properly configured",
    "error occurred while loading credentials",
    "unexpected credentials error",
    "access token provider was not enabled",
    "error occurred while loading an access token",
    "no credentials",
];

const NETWORK_MARKERS: &[&str] = &[
    "dispatch failure",
    "timed out",
    "timeout",
    "dns error",
    "connection refused",
    "connection reset",
    "network is unreachable",
    "io error",
];

pub fn classify(error: &anyhow::Error) -> FailureKind {
    let text = error_text(error);
    let has = |markers: &[&str]| markers.iter().any(|marker| text.contains(marker));

    if has(LOGIN_MARKERS) || (has(CREDENTIAL_LOADING_MARKERS) && !has(NETWORK_MARKERS)) {
        FailureKind::NeedsLogin
    } else {
        FailureKind::Transient
    }
}

fn error_text(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| format!("{} {:?}", cause, cause))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{classify, FailureKind};
    use anyhow::{anyhow, Context};
    use aws_sdk_ssm::error::ErrorMetadata;
    use aws_sdk_ssm::operation::start_session::StartSessionError;

    fn service_error(code: &str, message: &str) -> anyhow::Error {
        let error = StartSessionError::generic(
            ErrorMetadata::builder().code(code).message(message).build(),
        );
        anyhow::Error::new(error).context("Unable to start a Session Manager session.")
    }

    #[test]
    fn treats_an_expired_session_token_as_needing_login() {
        let error = service_error(
            "ExpiredTokenException",
            "The security token included in the request is expired",
        );

        assert_eq!(classify(&error), FailureKind::NeedsLogin);
    }

    #[test]
    fn treats_an_unknown_access_key_as_needing_login() {
        let error = service_error(
            "UnrecognizedClientException",
            "The security token included in the request is invalid.",
        );

        assert_eq!(classify(&error), FailureKind::NeedsLogin);
    }

    #[test]
    fn treats_an_expired_sso_token_as_needing_login() {
        let error = anyhow!("the SSO token has expired and cannot be refreshed")
            .context("an error occurred while loading credentials")
            .context("Unable to look up i-0123456789abcdef0 in Systems Manager.");

        assert_eq!(classify(&error), FailureKind::NeedsLogin);
    }

    #[test]
    fn treats_missing_awsctl_credentials_as_needing_login() {
        let error = anyhow!("Generate credentials and try again.");

        assert_eq!(classify(&error), FailureKind::NeedsLogin);
    }

    #[test]
    fn treats_a_credentials_failure_caused_by_the_network_as_transient() {
        let error = anyhow!("dispatch failure: dns error: failed to lookup address")
            .context("an error occurred while loading credentials");

        assert_eq!(classify(&error), FailureKind::Transient);
    }

    #[test]
    fn treats_an_offline_bastion_as_transient() {
        let error = Err::<(), _>(anyhow!(
            "The SSM agent on i-0 is not online (status: ConnectionLost)."
        ))
        .context("context")
        .expect_err("fails");

        assert_eq!(classify(&error), FailureKind::Transient);
    }

    #[test]
    fn treats_a_throttled_request_as_transient() {
        let error = service_error("ThrottlingException", "Rate exceeded");

        assert_eq!(classify(&error), FailureKind::Transient);
    }
}
