//! Command間で共有する有限な外部Serviceを定義する。

mod clock;
mod http;

pub(crate) use clock::{Clock, SystemClock};
pub(crate) use http::{
    ConditionalTextResponse, HttpResponse, HttpService, HttpServiceConfig, HttpServiceError,
};
