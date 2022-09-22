#![allow(clippy::integer_arithmetic)]

pub mod nonblocking;
pub mod quic_client;

#[macro_use]
extern crate solana_metrics;

use {
    crate::{
        nonblocking::quic_client::{QuicClient, QuicTpuConnection as NonblockingQuicTpuConnection},
        quic_client::QuicTpuConnection as BlockingQuicTpuConnection,
    },
    solana_tpu_client::tpu_connection_cache::{
        BaseTpuConnection, ConnectionCache, ConnectionCacheStats,
    },
    std::{net::SocketAddr, sync::Arc},
};

struct Quic(Arc<QuicClient>);
impl BaseTpuConnection for Quic {
    type BlockingConnectionType = BlockingQuicTpuConnection;
    type NonblockingConnectionType = NonblockingQuicTpuConnection;

    fn create_a_thing(cache: &ConnectionCache<Quic>, addr: &SocketAddr) -> Self {
        let map = cache.map.write().unwrap();

        let (_to_create_connection, endpoint) =
            map.get(addr)
                .map_or((true, cache.create_endpoint(false)), |pool| {
                    (
                        pool.need_new_connection(cache.connection_pool_size),
                        pool.endpoint.clone(),
                    )
                });

        Self(Arc::new(QuicClient::new(
            endpoint.as_ref().unwrap().clone(),
            *addr,
            cache.compute_max_parallel_streams(),
        )))
    }

    fn new_blocking_connection(
        &self,
        _addr: SocketAddr,
        stats: Arc<ConnectionCacheStats>,
    ) -> BlockingQuicTpuConnection {
        BlockingQuicTpuConnection::new_with_client(self.0.clone(), stats).into()
    }

    fn new_nonblocking_connection(
        &self,
        _addr: SocketAddr,
        stats: Arc<ConnectionCacheStats>,
    ) -> NonblockingQuicTpuConnection {
        NonblockingQuicTpuConnection::new_with_client(self.0.clone(), stats).into()
    }
}
