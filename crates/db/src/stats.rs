//! Stats aggregation for `/stats` (oracle:
//! `src/modules/alac/repositories/stats.repository.ts`).

use welds::connections::Client;

use crate::DbError;

/// Oracle TopTrackStat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopTrackStat {
    pub apple_track_id: String,
    pub request_count: i64,
}

/// Oracle AlacStats (all counts rounded the same way).
#[derive(Debug, Clone)]
pub struct AlacStats {
    pub total_cached_tracks: i64,
    pub total_requests: i64,
    pub cache_hits: i64,
    pub cache_misses: i64,
    pub cache_hit_ratio: f64,
    pub avg_rip_duration_ms: i64,
    pub avg_cache_duration_ms: i64,
    pub total_failed_requests: i64,
    pub top_tracks: Vec<TopTrackStat>,
}

/// Stats queries over the requests + tracks tables.
#[derive(Clone)]
pub struct StatsRepository {
    client: welds::connections::postgres::PostgresClient,
}
impl StatsRepository {
    pub fn new(client: welds::connections::postgres::PostgresClient) -> Self {
        Self { client }
    }

    /// One round-trip aggregate matching the oracle's seven queries.
    pub async fn get_stats(&self) -> Result<AlacStats, DbError> {
        let rows = Client::fetch_rows(
            &self.client,
                "SELECT \
                 (SELECT COUNT(*) FROM tracks) AS total_cached_tracks, \
                 (SELECT COUNT(*) FROM requests) AS total_requests, \
                 (SELECT COUNT(*) FROM requests WHERE is_cache_hit) AS cache_hits, \
                 (SELECT COUNT(*) FROM requests WHERE status = 'failed') AS total_failed, \
                 (SELECT ROUND(AVG(duration_ms)) FROM requests WHERE NOT is_cache_hit AND status = 'completed') AS avg_rip, \
                 (SELECT ROUND(AVG(duration_ms)) FROM requests WHERE is_cache_hit AND status = 'completed') AS avg_cache",
                &[],
            )
            .await?;
        let Some(row) = rows.into_iter().next() else {
            return Err(DbError::Row("stats query returned no rows".to_owned()));
        };
        let total_cached_tracks: i64 = row
            .get("total_cached_tracks")
            .map_err(|error| DbError::Row(error.to_string()))?;
        let total_requests: i64 = row
            .get("total_requests")
            .map_err(|error| DbError::Row(error.to_string()))?;
        let cache_hits: i64 = row
            .get("cache_hits")
            .map_err(|error| DbError::Row(error.to_string()))?;
        let total_failed_requests: i64 = row
            .get("total_failed")
            .map_err(|error| DbError::Row(error.to_string()))?;
        let avg_rip: Option<f64> = row
            .get("avg_rip")
            .map_err(|error| DbError::Row(error.to_string()))?;
        let avg_cache: Option<f64> = row
            .get("avg_cache")
            .map_err(|error| DbError::Row(error.to_string()))?;
        let cache_misses = total_requests - cache_hits;
        // Oracle: Math.round((hits / total) * 100 * 10) / 10
        let cache_hit_ratio = if total_requests > 0 {
            (cache_hits as f64 / total_requests as f64 * 1000.0).round() / 10.0
        } else {
            0.0
        };

        let top_rows = Client::fetch_rows(
            &self.client,
            "SELECT apple_track_id, COUNT(*) AS request_count FROM requests \
                 WHERE status = 'completed' GROUP BY apple_track_id \
                 ORDER BY COUNT(*) DESC LIMIT 5",
            &[],
        )
        .await?;
        let mut top_tracks = Vec::new();
        for top in top_rows {
            let apple_track_id: String = top
                .get("apple_track_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let request_count: i64 = top
                .get("request_count")
                .map_err(|error| DbError::Row(error.to_string()))?;
            top_tracks.push(TopTrackStat {
                apple_track_id,
                request_count,
            });
        }

        Ok(AlacStats {
            total_cached_tracks,
            total_requests,
            cache_hits,
            cache_misses,
            cache_hit_ratio,
            avg_rip_duration_ms: avg_rip.map(f64::round).unwrap_or(0.0) as i64,
            avg_cache_duration_ms: avg_cache.map(f64::round).unwrap_or(0.0) as i64,
            total_failed_requests,
            top_tracks,
        })
    }
}
