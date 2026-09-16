//! Implementations of the Qobuz gateway.

pub mod hosted;
pub mod native;

pub use hosted::HostedWorkerAdapter;
pub use native::NativeRipperAdapter;
