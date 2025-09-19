use async_trait::async_trait;
use kb_core::{QueryRequest, QueryResponse, Citation};
use kb_error::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, instrument};

use crate::engine::{RagEngine, RagMeta, HealthStatus, EngineStats};
use crate::rerank::Reranker;

/// 混合检索引擎 - 结合多种检索方法
pub struct HybridRagEngine {
    /// 向量检索引擎
    vector_engine: Arc<dyn RagEngine>,
    /// 词汇检索引擎
    lexical_engine: Option<Arc<dyn RagEngine>>,
    /// 图检索引擎
    graph_engine: Option<Arc<dyn RagEngine>>,
    /// 重排器
    reranker: Option<Arc<dyn Reranker>>,
    /// 混合配置
    config: HybridConfig,
}

/// 混合检索配置
#[derive(Debug, Clone)]
pub struct HybridConfig {
    /// 向量检索权重
    pub vector_weight: f32,
    /// 词汇检索权重
    pub lexical_weight: f32,
    /// 图检索权重
    pub graph_weight: f32,
    /// 各引擎返回的结果数倍数（用于后续融合）
    pub retrieval_multiplier: f32,
    /// 最终返回结果数
    pub final_top_k: usize,
    /// 分数归一化方法
    pub score_normalization: ScoreNormalization,
    /// 结果融合策略
    pub fusion_strategy: FusionStrategy,
    /// 最小分数阈值
    pub min_score_threshold: f32,
    /// 是否去重
    pub enable_deduplication: bool,
}

/// 分数归一化方法
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScoreNormalization {
    /// 无归一化
    None,
    /// 分数缩放到 [0, 1]
    MinMax,
    /// Z-score 标准化
    ZScore,
    /// 排名归一化（基于排名位置）
    Rank,
}

/// 结果融合策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FusionStrategy {
    /// 线性加权组合
    WeightedSum,
    /// RRF (Reciprocal Rank Fusion)
    RRF { k: f32 },
    /// CombSUM (累加分数)
    CombSum,
    /// CombMNZ (考虑匹配引擎数量)
    CombMNZ,
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.6,
            lexical_weight: 0.3,
            graph_weight: 0.1,
            retrieval_multiplier: 2.0,
            final_top_k: 10,
            score_normalization: ScoreNormalization::MinMax,
            fusion_strategy: FusionStrategy::RRF { k: 60.0 },
            min_score_threshold: 0.1,
            enable_deduplication: true,
        }
    }
}

/// 检索结果与来源引擎信息
#[derive(Debug, Clone)]
struct EngineResult {
    citation: Citation,
    engine_type: String,
    original_rank: usize,
    normalized_score: f32,
}

impl HybridRagEngine {
    pub fn new(vector_engine: Arc<dyn RagEngine>, config: HybridConfig) -> Self {
        Self {
            vector_engine,
            lexical_engine: None,
            graph_engine: None,
            reranker: None,
            config,
        }
    }

    /// 添加词汇检索引擎
    pub fn with_lexical_engine(mut self, engine: Arc<dyn RagEngine>) -> Self {
        self.lexical_engine = Some(engine);
        self
    }

    /// 添加图检索引擎
    pub fn with_graph_engine(mut self, engine: Arc<dyn RagEngine>) -> Self {
        self.graph_engine = Some(engine);
        self
    }

    /// 添加重排器
    pub fn with_reranker(mut self, reranker: Arc<dyn Reranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    /// 执行混合检索
    #[instrument(skip(self, req))]
    async fn perform_hybrid_search(&self, req: &QueryRequest) -> Result<Vec<Citation>> {
        let search_top_k = ((req.top_k.unwrap_or(self.config.final_top_k as u16) as f32)
            * self.config.retrieval_multiplier) as u16;

        let mut all_results = Vec::new();
        let mut search_req = req.clone();
        search_req.top_k = Some(search_top_k);

        // 1. 向量检索
        debug!("执行向量检索");
        let vector_response = self.vector_engine.query(search_req.clone()).await?;
        for (rank, citation) in vector_response.citations.into_iter().enumerate() {
            all_results.push(EngineResult {
                citation,
                engine_type: "vector".to_string(),
                original_rank: rank,
                normalized_score: 0.0, // 将在后面归一化
            });
        }

        // 2. 词汇检索（如果可用）
        if let Some(ref lexical_engine) = self.lexical_engine {
            debug!("执行词汇检索");
            let lexical_response = lexical_engine.query(search_req.clone()).await?;
            for (rank, citation) in lexical_response.citations.into_iter().enumerate() {
                all_results.push(EngineResult {
                    citation,
                    engine_type: "lexical".to_string(),
                    original_rank: rank,
                    normalized_score: 0.0,
                });
            }
        }

        // 3. 图检索（如果可用）
        if let Some(ref graph_engine) = self.graph_engine {
            debug!("执行图检索");
            let graph_response = graph_engine.query(search_req).await?;
            for (rank, citation) in graph_response.citations.into_iter().enumerate() {
                all_results.push(EngineResult {
                    citation,
                    engine_type: "graph".to_string(),
                    original_rank: rank,
                    normalized_score: 0.0,
                });
            }
        }

        if all_results.is_empty() {
            return Ok(vec![]);
        }

        // 4. 分数归一化
        self.normalize_scores(&mut all_results)?;

        // 5. 结果融合
        let fused_results = self.fuse_results(&all_results)?;

        // 6. 去重（如果启用）
        let deduplicated = if self.config.enable_deduplication {
            self.deduplicate_results(fused_results)
        } else {
            fused_results
        };

        // 7. 重排（如果可用）
        let final_results = if let Some(ref reranker) = self.reranker {
            debug!("执行重排");
            reranker.rerank(&req.query, deduplicated).await?
        } else {
            deduplicated
        };

        // 8. 限制最终结果数量
        let mut limited_results = final_results;
        limited_results.truncate(self.config.final_top_k);

        debug!(
            total_retrieved = all_results.len(),
            final_count = limited_results.len(),
            "混合检索完成"
        );

        Ok(limited_results)
    }

    /// 归一化分数
    fn normalize_scores(&self, results: &mut [EngineResult]) -> Result<()> {
        // 按引擎类型分组
        let mut engine_groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, result) in results.iter().enumerate() {
            engine_groups
                .entry(result.engine_type.clone())
                .or_insert_with(Vec::new)
                .push(idx);
        }

        // 对每个引擎的结果单独归一化
        for indices in engine_groups.values() {
            let scores: Vec<f32> = indices.iter().map(|&i| results[i].citation.score).collect();

            match self.config.score_normalization {
                ScoreNormalization::None => {
                    for &idx in indices {
                        results[idx].normalized_score = results[idx].citation.score;
                    }
                }
                ScoreNormalization::MinMax => {
                    let min_score = scores.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                    let max_score = scores.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                    let range = max_score - min_score;

                    for &idx in indices {
                        if range > 0.0 {
                            results[idx].normalized_score =
                                (results[idx].citation.score - min_score) / range;
                        } else {
                            results[idx].normalized_score = 1.0;
                        }
                    }
                }
                ScoreNormalization::ZScore => {
                    let mean = scores.iter().sum::<f32>() / scores.len() as f32;
                    let variance = scores.iter()
                        .map(|s| (s - mean) * (s - mean))
                        .sum::<f32>() / scores.len() as f32;
                    let std_dev = variance.sqrt();

                    for &idx in indices {
                        if std_dev > 0.0 {
                            results[idx].normalized_score =
                                (results[idx].citation.score - mean) / std_dev;
                        } else {
                            results[idx].normalized_score = 0.0;
                        }
                    }
                }
                ScoreNormalization::Rank => {
                    // 基于排名的归一化：rank_score = 1 / (rank + 1)
                    for &idx in indices {
                        results[idx].normalized_score =
                            1.0 / (results[idx].original_rank as f32 + 1.0);
                    }
                }
            }
        }

        Ok(())
    }

    /// 融合来自不同引擎的结果
    fn fuse_results(&self, results: &[EngineResult]) -> Result<Vec<Citation>> {
        let mut citation_scores: HashMap<String, (Citation, f32, Vec<String>)> = HashMap::new();

        for result in results {
            let key = format!("{}#{}", result.citation.document_id, result.citation.chunk_id);

            let weight = match result.engine_type.as_str() {
                "vector" => self.config.vector_weight,
                "lexical" => self.config.lexical_weight,
                "graph" => self.config.graph_weight,
                _ => 1.0,
            };

            let (citation, combined_score, engines) = citation_scores
                .entry(key)
                .or_insert_with(|| (result.citation.clone(), 0.0, Vec::new()));

            // 根据融合策略计算分数
            let contribution = match self.config.fusion_strategy {
                FusionStrategy::WeightedSum => {
                    result.normalized_score * weight
                }
                FusionStrategy::RRF { k } => {
                    weight / (k + result.original_rank as f32 + 1.0)
                }
                FusionStrategy::CombSum => {
                    result.normalized_score
                }
                FusionStrategy::CombMNZ => {
                    result.normalized_score * weight
                }
            };

            *combined_score += contribution;
            engines.push(result.engine_type.clone());

            // 更新引用信息（保留最高分数的版本）
            if result.citation.score > citation.score {
                *citation = result.citation.clone();
            }
        }

        // CombMNZ 需要乘以匹配引擎数量
        if matches!(self.config.fusion_strategy, FusionStrategy::CombMNZ) {
            for (_, (_, score, engines)) in citation_scores.iter_mut() {
                *score *= engines.len() as f32;
            }
        }

        // 转换为最终结果并排序
        let mut final_results: Vec<Citation> = citation_scores
            .into_iter()
            .filter_map(|(_, (mut citation, score, _))| {
                if score >= self.config.min_score_threshold {
                    citation.score = score;
                    Some(citation)
                } else {
                    None
                }
            })
            .collect();

        final_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        Ok(final_results)
    }

    /// 去重结果
    fn deduplicate_results(&self, mut results: Vec<Citation>) -> Vec<Citation> {
        let mut seen = HashSet::new();
        results.retain(|citation| {
            let key = format!("{}#{}", citation.document_id, citation.chunk_id);
            seen.insert(key)
        });
        results
    }

    /// 获取混合检索统计信息
    pub async fn get_hybrid_stats(&self) -> Result<HybridStats> {
        let vector_stats = self.vector_engine.stats().await?;

        let lexical_stats = if let Some(ref engine) = self.lexical_engine {
            Some(engine.stats().await?)
        } else {
            None
        };

        let graph_stats = if let Some(ref engine) = self.graph_engine {
            Some(engine.stats().await?)
        } else {
            None
        };

        Ok(HybridStats {
            vector_stats,
            lexical_stats,
            graph_stats,
            total_engines: 1 + self.lexical_engine.is_some() as u8 + self.graph_engine.is_some() as u8,
            reranker_enabled: self.reranker.is_some(),
        })
    }
}

/// 混合检索统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridStats {
    pub vector_stats: EngineStats,
    pub lexical_stats: Option<EngineStats>,
    pub graph_stats: Option<EngineStats>,
    pub total_engines: u8,
    pub reranker_enabled: bool,
}

#[async_trait]
impl RagEngine for HybridRagEngine {
    #[instrument(skip(self, req))]
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        let start_time = std::time::Instant::now();

        // 执行混合检索
        let citations = self.perform_hybrid_search(&req).await?;

        if citations.is_empty() {
            return Ok(QueryResponse {
                answer: "没有找到相关的文档内容".to_string(),
                citations: vec![],
                contexts: vec![],
                mode: "hybrid".to_string(),
                latency_ms: start_time.elapsed().as_millis() as i64,
            });
        }

        // 使用向量引擎来生成回答（因为它有完整的 LLM 集成）
        let vector_response = self.vector_engine.query(QueryRequest {
            query: req.query.clone(),
            mode: Some("vector".to_string()),
            top_k: Some(citations.len() as u16),
            filters: req.filters.clone(),
            rerank: req.rerank,
            include_raw_matches: req.include_raw_matches,
            stream: req.stream,
        }).await?;

        // 替换引用结果为混合检索的结果
        let contexts: Vec<String> = citations.iter().map(|c| c.snippet.clone()).collect();

        Ok(QueryResponse {
            answer: vector_response.answer,
            citations,
            contexts,
            mode: "hybrid".to_string(),
            latency_ms: start_time.elapsed().as_millis() as i64,
        })
    }

    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        // 将文档添加到所有可用的引擎
        self.vector_engine.add_document_text_with_meta(document_id, text, page, meta.clone()).await?;

        if let Some(ref lexical_engine) = self.lexical_engine {
            lexical_engine.add_document_text_with_meta(document_id, text, page, meta.clone()).await?;
        }

        if let Some(ref graph_engine) = self.graph_engine {
            graph_engine.add_document_text_with_meta(document_id, text, page, meta).await?;
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        let mut issues = Vec::new();

        // 检查向量引擎
        if let Err(e) = self.vector_engine.health_check().await {
            issues.push(format!("Vector engine: {}", e));
        }

        // 检查词汇引擎
        if let Some(ref engine) = self.lexical_engine {
            if let Err(e) = engine.health_check().await {
                issues.push(format!("Lexical engine: {}", e));
            }
        }

        // 检查图引擎
        if let Some(ref engine) = self.graph_engine {
            if let Err(e) = engine.health_check().await {
                issues.push(format!("Graph engine: {}", e));
            }
        }

        // 检查重排器
        if let Some(ref reranker) = self.reranker {
            if let Err(e) = reranker.health_check().await {
                issues.push(format!("Reranker: {}", e));
            }
        }

        if issues.is_empty() {
            Ok(HealthStatus::Healthy)
        } else if issues.len() == 1 {
            Ok(HealthStatus::Degraded {
                reason: format!("Components with issues: {}", issues.join(", ")),
            })
        } else {
            Ok(HealthStatus::Unhealthy {
                error: format!("Multiple component failures: {}", issues.join(", ")),
            })
        }
    }

    async fn stats(&self) -> Result<EngineStats> {
        let hybrid_stats = self.get_hybrid_stats().await?;

        // 聚合所有引擎的统计信息
        let mut total_documents = hybrid_stats.vector_stats.total_documents;
        let mut total_chunks = hybrid_stats.vector_stats.total_chunks;
        let mut index_size_bytes = hybrid_stats.vector_stats.index_size_bytes;

        if let Some(ref lexical_stats) = hybrid_stats.lexical_stats {
            total_documents = total_documents.max(lexical_stats.total_documents);
            total_chunks = total_chunks.max(lexical_stats.total_chunks);
            index_size_bytes += lexical_stats.index_size_bytes;
        }

        if let Some(ref graph_stats) = hybrid_stats.graph_stats {
            total_documents = total_documents.max(graph_stats.total_documents);
            total_chunks = total_chunks.max(graph_stats.total_chunks);
            index_size_bytes += graph_stats.index_size_bytes;
        }

        Ok(EngineStats {
            total_documents,
            total_chunks,
            index_size_bytes,
            last_updated: hybrid_stats.vector_stats.last_updated,
            query_count: hybrid_stats.vector_stats.query_count,
            average_query_latency_ms: hybrid_stats.vector_stats.average_query_latency_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{NoopRagEngine};

    #[tokio::test]
    async fn test_hybrid_config_default() {
        let config = HybridConfig::default();
        assert_eq!(config.vector_weight + config.lexical_weight + config.graph_weight, 1.0);
        assert!(config.final_top_k > 0);
    }

    #[tokio::test]
    async fn test_hybrid_engine_creation() {
        let vector_engine = Arc::new(NoopRagEngine);
        let config = HybridConfig::default();
        let hybrid_engine = HybridRagEngine::new(vector_engine, config);

        let health = hybrid_engine.health_check().await.unwrap();
        assert!(matches!(health, HealthStatus::Healthy));
    }

    #[tokio::test]
    async fn test_deduplicate_results() {
        let vector_engine = Arc::new(NoopRagEngine);
        let config = HybridConfig::default();
        let hybrid_engine = HybridRagEngine::new(vector_engine, config);

        let citations = vec![
            Citation {
                document_id: "doc1".to_string(),
                chunk_id: "chunk1".to_string(),
                page: None,
                score: 0.9,
                snippet: "test".to_string(),
            },
            Citation {
                document_id: "doc1".to_string(),
                chunk_id: "chunk1".to_string(),
                page: None,
                score: 0.8,
                snippet: "test".to_string(),
            },
        ];

        let deduplicated = hybrid_engine.deduplicate_results(citations);
        assert_eq!(deduplicated.len(), 1);
    }
}