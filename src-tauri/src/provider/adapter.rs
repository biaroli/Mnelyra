use std::future::Future;
use std::pin::Pin;

use super::{ProviderDescriptor, ProviderStatus};

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ProviderAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn status(&self) -> ProviderFuture<'_, ProviderStatus>;
}
