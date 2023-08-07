#![feature(slice_ptr_get)]

mod priv_prelude;
mod namespace;
mod machine;
mod iface;
mod ioctl;
mod network;
mod connect;
mod adapter;
mod stream_ext;

pub use {
    machine::Machine,
    iface::create::{IpPacketStream, IpPacketSink},
    connect::{connect, Connect},
};
