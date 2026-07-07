//! Payments domain — the payments bounded context's core.
//!
//! This crate holds the payment state machine, the [`PaymentRepository`]
//! persistence port, and the [`PaymentProvider`] gateway port. It depends on
//! nothing but the standard library plus `async-trait` (an ergonomics macro,
//! not a runtime) — no web framework, no database driver, and critically no
//! payment-provider SDK. That purity is what "changing provider does not
//! touch the domain or public API" means in practice: a concrete gateway
//! (Stripe, a fake for tests, ...) lives entirely in its own adapter crate and
//! is reachable only through [`PaymentProvider`]; the Postgres adapter behind
//! [`PaymentRepository`] lives in `infrastructure`.

mod money;
mod payment;
mod provider;

pub use money::{Currency, CurrencyError, Money, MoneyError};
pub use payment::{
    NewPayment, Payment, PaymentId, PaymentRepository, PaymentRepositoryError, PaymentStatus,
    PaymentStatusChange, PaymentStatusError, ProviderReference,
};
pub use provider::{PaymentProvider, ProviderError, ProviderIntent};
