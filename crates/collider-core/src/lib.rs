//! collider-core: the capture engine behind the `collider` CLI.
//!
//! ```no_run
//! # async fn run() -> collider_core::Result<()> {
//! use std::{sync::Arc, time::Duration};
//! use collider_core::{Client, Downloader, Options};
//!
//! let client = Client::new(&[], 5, Duration::from_secs(30))?;
//! let dl = Downloader::new(client, Options::default(), Arc::new(|_| {}), Default::default());
//! dl.download("https://example.com/manifest.mpd", "out.mp4".as_ref()).await?;
//! # Ok(()) }
//! ```

pub mod crypto;
pub mod dash;
pub mod downloader;
pub mod error;
pub mod hls;
pub mod http;
pub mod model;
pub mod mux;
pub mod sidx;
pub mod source;

pub use downloader::{Downloader, Event, Options, Reporter};
pub use error::{Error, Result};
pub use hls::Quality;
pub use http::Client;
pub use model::{Protocol, StreamInfo};
