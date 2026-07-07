//! The OpenAPI document for the JSON API (bead authapp-a05b92).
//!
//! Derived from the handlers' `#[utoipa::path]` annotations and the
//! request/response types' `#[derive(ToSchema)]` — there is no hand-maintained
//! duplicate of the contract. The `server openapi` subcommand serializes
//! [`ApiDoc`] to JSON, and the monorepo generates the TypeScript client from
//! that spec, so front and back share one source of truth.
//!
//! The spec covers the JSON API a typed client consumes (auth, audit, health,
//! transactions). The OAuth endpoints are browser redirects and the payment
//! webhook is provider-facing, so neither is a typed-client surface and both
//! are left out.

use utoipa::OpenApi;

/// The API's OpenAPI 3.1 document.
#[derive(Debug, OpenApi)]
#[openapi(
    info(
        title = "auth-app API",
        description = "The JSON API the admin frontend consumes.",
        version = "0.1.0",
    ),
    paths(
        crate::health_handler,
        crate::auth::login_handler,
        crate::session::me_handler,
        crate::session::logout_handler,
        crate::oauth::providers_handler,
        crate::audit::list_events,
        crate::transactions::list_transactions,
        crate::transactions::get_transaction,
        crate::transactions::refund_transaction,
    ),
    components(schemas(
        crate::error::ErrorResponse,
        crate::auth::LoginBody,
        crate::auth::LoginOk,
        crate::session::MeOut,
        crate::oauth::ProvidersOut,
        crate::audit::EventOut,
        crate::transactions::TransactionOut,
        crate::transactions::TransactionPage,
        crate::transactions::StatusChangeOut,
        crate::transactions::TransactionDetailOut,
        crate::transactions::RefundOut,
    )),
    tags(
        (name = "auth", description = "Sign-in, session, and identity"),
        (name = "audit", description = "Authentication audit trail"),
        (name = "transactions", description = "Payment transactions and refunds"),
        (name = "health", description = "Readiness probe"),
    ),
)]
pub struct ApiDoc;

impl ApiDoc {
    /// The spec as pretty-printed JSON — what `server openapi` writes and the
    /// client generator reads.
    pub fn to_pretty_json() -> String {
        <Self as OpenApi>::openapi()
            .to_pretty_json()
            .expect("serializing the OpenAPI document cannot fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_lists_every_documented_path() {
        let json = ApiDoc::to_pretty_json();
        for path in [
            "/health",
            "/auth/login",
            "/auth/me",
            "/auth/logout",
            "/auth/oauth/providers",
            "/audit/events",
            "/transactions",
            "/transactions/{id}",
            "/transactions/{id}/refund",
        ] {
            assert!(json.contains(path), "spec is missing {path}");
        }
        // A schema the client's types come from.
        assert!(json.contains("LoginOk"));
        assert!(json.contains("TransactionPage"));
    }
}
