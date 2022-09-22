#![allow(clippy::integer_arithmetic)]

pub mod nonblocking;
pub mod udp_client;

use {
    crate::{
        nonblocking::udp_client::UdpTpuConnection as NonblockingUdpTpuConnection,
        udp_client::UdpTpuConnection as BlockingUdpTpuConnection,
    },
    solana_tpu_client::tpu_connection_cache::{
        BaseTpuConnection, ConnectionCache, ConnectionCacheStats,
    },
    std::{
        net::{SocketAddr, UdpSocket},
        sync::Arc,
    },
};

struct Udp(Arc<UdpSocket>);
impl BaseTpuConnection for Udp {
    type BlockingConnectionType = BlockingUdpTpuConnection;
    type NonblockingConnectionType = NonblockingUdpTpuConnection;

    fn create_a_thing(cache: &ConnectionCache<Udp>, _addr: &SocketAddr) -> Self {
        Self(cache.tpu_udp_socket.clone())
    }

    fn new_blocking_connection(
        &self,
        addr: SocketAddr,
        _stats: Arc<ConnectionCacheStats>,
    ) -> BlockingUdpTpuConnection {
        BlockingUdpTpuConnection::new_from_addr(self.0.clone(), addr).into()
    }

    fn new_nonblocking_connection(
        &self,
        addr: SocketAddr,
        _stats: Arc<ConnectionCacheStats>,
    ) -> NonblockingUdpTpuConnection {
        NonblockingUdpTpuConnection::new_from_addr(self.0.try_clone().unwrap(), addr).into()
    }
}
