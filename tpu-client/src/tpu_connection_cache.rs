use {
    crate::tpu_connection::ClientStats,
    indexmap::map::IndexMap,
    rand::{thread_rng, Rng},
    solana_measure::measure::Measure,
    solana_sdk::timing::AtomicInterval,
    std::{
        net::SocketAddr,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, RwLock,
        },
    },
};

// Should be non-zero
static MAX_CONNECTIONS: usize = 1024;

/// Used to decide whether the TPU and underlying connection cache should use
/// QUIC connections.
pub const DEFAULT_TPU_USE_QUIC: bool = true;

/// Default TPU connection pool size per remote address
pub const DEFAULT_TPU_CONNECTION_POOL_SIZE: usize = 4;

pub const DEFAULT_TPU_ENABLE_UDP: bool = false;

#[derive(Default)]
pub struct ConnectionCacheStats {
    pub(crate) cache_hits: AtomicU64,
    pub(crate) cache_misses: AtomicU64,
    pub(crate) cache_evictions: AtomicU64,
    pub(crate) eviction_time_ms: AtomicU64,
    pub(crate) sent_packets: AtomicU64,
    pub(crate) total_batches: AtomicU64,
    pub(crate) batch_success: AtomicU64,
    pub(crate) batch_failure: AtomicU64,
    pub(crate) get_connection_ms: AtomicU64,
    pub(crate) get_connection_lock_ms: AtomicU64,
    pub(crate) get_connection_hit_ms: AtomicU64,
    pub(crate) get_connection_miss_ms: AtomicU64,

    // Need to track these separately per-connection
    // because we need to track the base stat value from quinn
    pub total_client_stats: ClientStats,
}

pub const CONNECTION_STAT_SUBMISSION_INTERVAL: u64 = 2000;

impl ConnectionCacheStats {
    pub fn add_client_stats(
        &self,
        client_stats: &ClientStats,
        num_packets: usize,
        is_success: bool,
    ) {
        self.total_client_stats.total_connections.fetch_add(
            client_stats.total_connections.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.total_client_stats.connection_reuse.fetch_add(
            client_stats.connection_reuse.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.total_client_stats.connection_errors.fetch_add(
            client_stats.connection_errors.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.total_client_stats.zero_rtt_accepts.fetch_add(
            client_stats.zero_rtt_accepts.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.total_client_stats.zero_rtt_rejects.fetch_add(
            client_stats.zero_rtt_rejects.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.total_client_stats.make_connection_ms.fetch_add(
            client_stats.make_connection_ms.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.sent_packets
            .fetch_add(num_packets as u64, Ordering::Relaxed);
        self.total_batches.fetch_add(1, Ordering::Relaxed);
        if is_success {
            self.batch_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.batch_failure.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn report(&self) {
        datapoint_info!(
            "quic-client-connection-stats",
            (
                "cache_hits",
                self.cache_hits.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "cache_misses",
                self.cache_misses.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "cache_evictions",
                self.cache_evictions.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "eviction_time_ms",
                self.eviction_time_ms.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "get_connection_ms",
                self.get_connection_ms.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "get_connection_lock_ms",
                self.get_connection_lock_ms.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "get_connection_hit_ms",
                self.get_connection_hit_ms.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "get_connection_miss_ms",
                self.get_connection_miss_ms.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "make_connection_ms",
                self.total_client_stats
                    .make_connection_ms
                    .swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "total_connections",
                self.total_client_stats
                    .total_connections
                    .swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "connection_reuse",
                self.total_client_stats
                    .connection_reuse
                    .swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "connection_errors",
                self.total_client_stats
                    .connection_errors
                    .swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "zero_rtt_accepts",
                self.total_client_stats
                    .zero_rtt_accepts
                    .swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "zero_rtt_rejects",
                self.total_client_stats
                    .zero_rtt_rejects
                    .swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "congestion_events",
                self.total_client_stats.congestion_events.load_and_reset(),
                i64
            ),
            (
                "tx_streams_blocked_uni",
                self.total_client_stats
                    .tx_streams_blocked_uni
                    .load_and_reset(),
                i64
            ),
            (
                "tx_data_blocked",
                self.total_client_stats.tx_data_blocked.load_and_reset(),
                i64
            ),
            (
                "tx_acks",
                self.total_client_stats.tx_acks.load_and_reset(),
                i64
            ),
            (
                "num_packets",
                self.sent_packets.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "total_batches",
                self.total_batches.swap(0, Ordering::Relaxed),
                i64
            ),
            (
                "batch_failure",
                self.batch_failure.swap(0, Ordering::Relaxed),
                i64
            ),
        );
    }
}

pub struct ConnectionCache<P: ConnectionPool> {
    pub map: RwLock<IndexMap<SocketAddr, P>>,
    pub stats: Arc<ConnectionCacheStats>,
    pub last_stats: AtomicInterval,
    pub connection_pool_size: usize,
    pub tpu_config: P::TpuConfig,
}

impl<P: ConnectionPool> ConnectionCache<P> {
    pub fn new(connection_pool_size: usize) -> Self {
        Self {
            connection_pool_size: 1.max(connection_pool_size), // The minimum pool size is 1.
            ..Self::default()
        }
    }

    /// Create a lazy connection object under the exclusive lock of the cache map if there is not
    /// enough used connections in the connection pool for the specified address.
    /// Returns CreateConnectionResult.
    fn create_connection(
        &self,
        lock_timing_ms: &mut u64,
        addr: &SocketAddr,
    ) -> CreateConnectionResult<P::PoolTpuConnection> {
        let mut get_connection_map_lock_measure = Measure::start("get_connection_map_lock_measure");
        let mut map = self.map.write().unwrap();
        get_connection_map_lock_measure.stop();
        *lock_timing_ms = lock_timing_ms.saturating_add(get_connection_map_lock_measure.as_ms());
        // Read again, as it is possible that between read lock dropped and the write lock acquired
        // another thread could have setup the connection.

        let should_create_connection = map
            .get(addr)
            .map(|pool| pool.need_new_connection(self.connection_pool_size))
            .unwrap_or(true);

        let (cache_hit, num_evictions, eviction_timing_ms) = if should_create_connection {
            // evict a connection if the cache is reaching upper bounds
            let mut num_evictions = 0;
            let mut get_connection_cache_eviction_measure =
                Measure::start("get_connection_cache_eviction_measure");
            while map.len() >= MAX_CONNECTIONS {
                let mut rng = thread_rng();
                let n = rng.gen_range(0, MAX_CONNECTIONS);
                map.swap_remove_index(n);
                num_evictions += 1;
            }
            get_connection_cache_eviction_measure.stop();

            map.entry(*addr)
                .and_modify(|pool| {
                    pool.add_connection(&self.tpu_config, addr);
                })
                .or_insert_with(|| P::new_with_connection(&self.tpu_config, addr));

            (
                false,
                num_evictions,
                get_connection_cache_eviction_measure.as_ms(),
            )
        } else {
            (true, 0, 0)
        };

        let pool = map.get(addr).unwrap();
        let connection = pool.borrow_connection();

        CreateConnectionResult {
            connection,
            cache_hit,
            connection_cache_stats: self.stats.clone(),
            num_evictions,
            eviction_timing_ms,
        }
    }

    fn get_or_add_connection(
        &self,
        addr: &SocketAddr,
    ) -> GetConnectionResult<P::PoolTpuConnection> {
        let mut get_connection_map_lock_measure = Measure::start("get_connection_map_lock_measure");
        let map = self.map.read().unwrap();
        get_connection_map_lock_measure.stop();

        let port_offset = P::PORT_OFFSET;

        let port = addr
            .port()
            .checked_add(port_offset)
            .unwrap_or_else(|| addr.port());
        let addr = SocketAddr::new(addr.ip(), port);

        let mut lock_timing_ms = get_connection_map_lock_measure.as_ms();

        let report_stats = self
            .last_stats
            .should_update(CONNECTION_STAT_SUBMISSION_INTERVAL);

        let mut get_connection_map_measure = Measure::start("get_connection_hit_measure");
        let CreateConnectionResult {
            connection,
            cache_hit,
            connection_cache_stats,
            num_evictions,
            eviction_timing_ms,
        } = match map.get(&addr) {
            Some(pool) => {
                if pool.need_new_connection(self.connection_pool_size) {
                    // create more connection and put it in the pool
                    drop(map);
                    self.create_connection(&mut lock_timing_ms, &addr)
                } else {
                    let connection = pool.borrow_connection();
                    CreateConnectionResult {
                        connection,
                        cache_hit: true,
                        connection_cache_stats: self.stats.clone(),
                        num_evictions: 0,
                        eviction_timing_ms: 0,
                    }
                }
            }
            None => {
                // Upgrade to write access by dropping read lock and acquire write lock
                drop(map);
                self.create_connection(&mut lock_timing_ms, &addr)
            }
        };
        get_connection_map_measure.stop();

        GetConnectionResult {
            connection,
            cache_hit,
            report_stats,
            map_timing_ms: get_connection_map_measure.as_ms(),
            lock_timing_ms,
            connection_cache_stats,
            num_evictions,
            eviction_timing_ms,
        }
    }

    fn get_connection_and_log_stats(
        &self,
        addr: &SocketAddr,
    ) -> (Arc<P::PoolTpuConnection>, Arc<ConnectionCacheStats>) {
        let mut get_connection_measure = Measure::start("get_connection_measure");
        let GetConnectionResult {
            connection,
            cache_hit,
            report_stats,
            map_timing_ms,
            lock_timing_ms,
            connection_cache_stats,
            num_evictions,
            eviction_timing_ms,
        } = self.get_or_add_connection(addr);

        if report_stats {
            connection_cache_stats.report();
        }

        if cache_hit {
            connection_cache_stats
                .cache_hits
                .fetch_add(1, Ordering::Relaxed);
            connection_cache_stats
                .get_connection_hit_ms
                .fetch_add(map_timing_ms, Ordering::Relaxed);
        } else {
            connection_cache_stats
                .cache_misses
                .fetch_add(1, Ordering::Relaxed);
            connection_cache_stats
                .get_connection_miss_ms
                .fetch_add(map_timing_ms, Ordering::Relaxed);
            connection_cache_stats
                .cache_evictions
                .fetch_add(num_evictions, Ordering::Relaxed);
            connection_cache_stats
                .eviction_time_ms
                .fetch_add(eviction_timing_ms, Ordering::Relaxed);
        }

        get_connection_measure.stop();
        connection_cache_stats
            .get_connection_lock_ms
            .fetch_add(lock_timing_ms, Ordering::Relaxed);
        connection_cache_stats
            .get_connection_ms
            .fetch_add(get_connection_measure.as_ms(), Ordering::Relaxed);

        (connection, connection_cache_stats)
    }

    pub fn get_connection(
        &self,
        addr: &SocketAddr,
    ) -> <P::PoolTpuConnection as BaseTpuConnection>::BlockingConnectionType {
        let (connection, connection_cache_stats) = self.get_connection_and_log_stats(addr);
        connection.new_blocking_connection(*addr, connection_cache_stats)
    }

    pub fn get_nonblocking_connection(
        &self,
        addr: &SocketAddr,
    ) -> <P::PoolTpuConnection as BaseTpuConnection>::NonblockingConnectionType {
        let (connection, connection_cache_stats) = self.get_connection_and_log_stats(addr);
        connection.new_nonblocking_connection(*addr, connection_cache_stats)
    }
}

impl<P: ConnectionPool> Default for ConnectionCache<P> {
    fn default() -> Self {
        Self {
            map: RwLock::new(IndexMap::with_capacity(MAX_CONNECTIONS)),
            stats: Arc::new(ConnectionCacheStats::default()),
            last_stats: AtomicInterval::default(),
            connection_pool_size: DEFAULT_TPU_CONNECTION_POOL_SIZE,
            tpu_config: P::TpuConfig::default(),
        }
    }
}

pub trait ConnectionPool {
    type PoolTpuConnection: BaseTpuConnection;
    type TpuConfig: Default;
    const PORT_OFFSET: u16 = 0;

    fn new_with_connection(config: &Self::TpuConfig, addr: &SocketAddr) -> Self;

    fn add_connection(&mut self, config: &Self::TpuConfig, addr: &SocketAddr);

    fn num_connections(&self) -> usize;

    fn get_connection_unchecked(&self, index: usize) -> Arc<Self::PoolTpuConnection>;

    /// Get a connection from the pool. It must have at least one connection in the pool.
    /// This randomly picks a connection in the pool.
    fn borrow_connection(&self) -> Arc<Self::PoolTpuConnection> {
        let mut rng = thread_rng();
        let n = rng.gen_range(0, self.num_connections());
        self.get_connection_unchecked(n)
    }
    /// Check if we need to create a new connection. If the count of the connections
    /// is smaller than the pool size.
    fn need_new_connection(&self, required_pool_size: usize) -> bool {
        self.num_connections() < required_pool_size
    }

    fn create_pool_entry(
        &self,
        config: &Self::TpuConfig,
        addr: &SocketAddr,
    ) -> Self::PoolTpuConnection;
}

pub trait BaseTpuConnection {
    type BlockingConnectionType;
    type NonblockingConnectionType;

    fn new_blocking_connection(
        &self,
        addr: SocketAddr,
        stats: Arc<ConnectionCacheStats>,
    ) -> Self::BlockingConnectionType;

    fn new_nonblocking_connection(
        &self,
        addr: SocketAddr,
        stats: Arc<ConnectionCacheStats>,
    ) -> Self::NonblockingConnectionType;
}

struct GetConnectionResult<T: BaseTpuConnection> {
    connection: Arc<T>,
    cache_hit: bool,
    report_stats: bool,
    map_timing_ms: u64,
    lock_timing_ms: u64,
    connection_cache_stats: Arc<ConnectionCacheStats>,
    num_evictions: u64,
    eviction_timing_ms: u64,
}

struct CreateConnectionResult<T: BaseTpuConnection> {
    connection: Arc<T>,
    cache_hit: bool,
    connection_cache_stats: Arc<ConnectionCacheStats>,
    num_evictions: u64,
    eviction_timing_ms: u64,
}

#[cfg(test)]
mod tests {
    use {
        crate::{
            connection_cache::{ConnectionCache, MAX_CONNECTIONS},
            tpu_connection::TpuConnection,
        },
        rand::{Rng, SeedableRng},
        rand_chacha::ChaChaRng,
        solana_sdk::{
            pubkey::Pubkey,
            quic::{
                QUIC_MAX_UNSTAKED_CONCURRENT_STREAMS, QUIC_MIN_STAKED_CONCURRENT_STREAMS,
                QUIC_PORT_OFFSET,
            },
        },
        solana_streamer::streamer::StakedNodes,
        std::{
            net::{IpAddr, Ipv4Addr, SocketAddr},
            sync::{Arc, RwLock},
        },
    };

    fn get_addr(rng: &mut ChaChaRng) -> SocketAddr {
        let a = rng.gen_range(1, 255);
        let b = rng.gen_range(1, 255);
        let c = rng.gen_range(1, 255);
        let d = rng.gen_range(1, 255);

        let addr_str = format!("{}.{}.{}.{}:80", a, b, c, d);

        addr_str.parse().expect("Invalid address")
    }

    #[test]
    fn test_connection_cache() {
        solana_logger::setup();
        // Allow the test to run deterministically
        // with the same pseudorandom sequence between runs
        // and on different platforms - the cryptographic security
        // property isn't important here but ChaChaRng provides a way
        // to get the same pseudorandom sequence on different platforms
        let mut rng = ChaChaRng::seed_from_u64(42);

        // Generate a bunch of random addresses and create TPUConnections to them
        // Since TPUConnection::new is infallible, it should't matter whether or not
        // we can actually connect to those addresses - TPUConnection implementations should either
        // be lazy and not connect until first use or handle connection errors somehow
        // (without crashing, as would be required in a real practical validator)
        let connection_cache = ConnectionCache::default();
        let port_offset = if connection_cache.use_quic() {
            QUIC_PORT_OFFSET
        } else {
            0
        };
        let addrs = (0..MAX_CONNECTIONS)
            .into_iter()
            .map(|_| {
                let addr = get_addr(&mut rng);
                connection_cache.get_connection(&addr);
                addr
            })
            .collect::<Vec<_>>();
        {
            let map = connection_cache.map.read().unwrap();
            assert!(map.len() == MAX_CONNECTIONS);
            addrs.iter().for_each(|a| {
                let port = a
                    .port()
                    .checked_add(port_offset)
                    .unwrap_or_else(|| a.port());
                let addr = &SocketAddr::new(a.ip(), port);

                let conn = &map.get(addr).expect("Address not found").connections[0];
                let conn = conn.new_blocking_connection(*addr, connection_cache.stats.clone());
                assert!(addr.ip() == conn.tpu_addr().ip());
            });
        }

        let addr = &get_addr(&mut rng);
        connection_cache.get_connection(addr);

        let port = addr
            .port()
            .checked_add(port_offset)
            .unwrap_or_else(|| addr.port());
        let addr_with_quic_port = SocketAddr::new(addr.ip(), port);
        let map = connection_cache.map.read().unwrap();
        assert!(map.len() == MAX_CONNECTIONS);
        let _conn = map.get(&addr_with_quic_port).expect("Address not found");
    }

    #[test]
    fn test_connection_cache_max_parallel_chunks() {
        solana_logger::setup();
        let mut connection_cache = ConnectionCache::default();
        assert_eq!(
            connection_cache.compute_max_parallel_streams(),
            QUIC_MAX_UNSTAKED_CONCURRENT_STREAMS
        );

        let staked_nodes = Arc::new(RwLock::new(StakedNodes::default()));
        let pubkey = Pubkey::new_unique();
        connection_cache.set_staked_nodes(&staked_nodes, &pubkey);
        assert_eq!(
            connection_cache.compute_max_parallel_streams(),
            QUIC_MAX_UNSTAKED_CONCURRENT_STREAMS
        );

        staked_nodes.write().unwrap().total_stake = 10000;
        assert_eq!(
            connection_cache.compute_max_parallel_streams(),
            QUIC_MAX_UNSTAKED_CONCURRENT_STREAMS
        );

        staked_nodes
            .write()
            .unwrap()
            .pubkey_stake_map
            .insert(pubkey, 1);
        assert_eq!(
            connection_cache.compute_max_parallel_streams(),
            QUIC_MIN_STAKED_CONCURRENT_STREAMS
        );

        staked_nodes
            .write()
            .unwrap()
            .pubkey_stake_map
            .remove(&pubkey);
        staked_nodes
            .write()
            .unwrap()
            .pubkey_stake_map
            .insert(pubkey, 1000);
        assert_ne!(
            connection_cache.compute_max_parallel_streams(),
            QUIC_MIN_STAKED_CONCURRENT_STREAMS
        );
    }

    // Test that we can get_connection with a connection cache configured for quic
    // on an address with a port that, if QUIC_PORT_OFFSET were added to it, it would overflow to
    // an invalid port.
    #[test]
    fn test_overflow_address() {
        let port = u16::MAX - QUIC_PORT_OFFSET + 1;
        assert!(port.checked_add(QUIC_PORT_OFFSET).is_none());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);
        let connection_cache = ConnectionCache::new(1);

        let conn = connection_cache.get_connection(&addr);
        // We (intentionally) don't have an interface that allows us to distinguish between
        // UDP and Quic connections, so check instead that the port is valid (non-zero)
        // and is the same as the input port (falling back on UDP)
        assert!(conn.tpu_addr().port() != 0);
        assert!(conn.tpu_addr().port() == port);
    }
}
