//! Native wrapper-lite engine module for Apple Music ALAC decryption.

pub mod cenc;
pub mod client;
pub mod decryptor;
pub mod engine;
pub mod playlist;
pub mod widevine;

pub use client::{WrapperError, WrapperLiteClient};
pub use decryptor::{decrypt_fragment, transform_init_segment};
pub use engine::WrapperEngine;
pub use playlist::{
    parse_master_playlist, parse_media_playlist, AlacStreamInfo, CodecPreference, MediaPlaylistInfo,
};
