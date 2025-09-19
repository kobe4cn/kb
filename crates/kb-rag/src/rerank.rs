use async_trait::async_trait;
use kb_core::Citation;
use kb_error::{KbError, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// 重排器抽象接口
#[async_trait]
pub trait Reranker: Send + Sync {
    /// 重新排序检索结果
    async fn rerank(&self, query: &str, results: Vec<Citation>) -> Result<Vec<Citation>>;

    /// 获取重排器名称
    fn name(&self) -> &str;

    /// 健康检查
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

/// 基于关键词重叠的简单重排器
pub struct KeywordReranker {
    name: String,
    case_sensitive: bool,
    boost_factor: f32,
}

impl KeywordReranker {
    pub fn new(case_sensitive: bool, boost_factor: f32) -> Self {
        Self {
            name: "keyword".to_string(),
            case_sensitive,
            boost_factor,
        }
    }

    fn calculate_keyword_score(&self, query: &str, text: &str) -> f32 {
        let (query_words, query_lower): (Vec<&str>, Option<String>) = if self.case_sensitive {
            (query.split_whitespace().collect(), None)
        } else {
            let lower = query.to_lowercase();
            (vec![], Some(lower))
        };

        let query_words = if let Some(ref lower) = query_lower {
            lower.split_whitespace().collect()
        } else {
            query_words
        };

        let text_content = if self.case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };

        let matches: usize = query_words
            .iter()
            .filter(|&&word| text_content.contains(word))
            .count();

        if query_words.is_empty() {
            0.0
        } else {
            (matches as f32 / query_words.len() as f32) * self.boost_factor
        }
    }
}

#[async_trait]
impl Reranker for KeywordReranker {
    #[instrument(skip(self, results))]
    async fn rerank(&self, query: &str, mut results: Vec<Citation>) -> Result<Vec<Citation>> {
        // 计算每个结果的关键词匹配分数
        for citation in &mut results {
            let keyword_score = self.calculate_keyword_score(query, &citation.snippet);
            // 将关键词分数与原始相似度分数结合
            citation.score = citation.score * (1.0 + keyword_score);
        }

        // 按新分数排序
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        tracing::debug!(
            query = %query,
            results_count = results.len(),
            "Keyword reranking completed"
        );

        Ok(results)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Cohere API 重排器
pub struct CohereReranker {
    name: String,
    client: reqwest::Client,
    api_key: String,
    model: String,
    api_url: String,
    max_chunks_per_request: usize,
}

/// 语义相似度重排器 - 使用嵌入模型计算语义相似度
pub struct SemanticReranker {
    name: String,
    embed_model: std::sync::Arc<dyn kb_llm::EmbedModel>,
    similarity_threshold: f32,
    boost_factor: f32,
}

/// 长度奖励重排器 - 根据文档长度给予奖励或惩罚
pub struct LengthReranker {
    name: String,
    optimal_length: usize,
    length_penalty_factor: f32,
}

/// 多样性重排器 - 基于最大边际相关性(MMR)的多样性重排
#[allow(dead_code)]
pub struct DiversityReranker {
    name: String,
    embed_model: std::sync::Arc<dyn kb_llm::EmbedModel>,
    lambda: f32, // 相关性与多样性的权衡参数 (0-1)
    similarity_threshold: f32,
}

impl CohereReranker {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            name: "cohere".to_string(),
            client: reqwest::Client::new(),
            api_key,
            model: model.unwrap_or_else(|| "rerank-multilingual-v3.0".to_string()),
            api_url: "https://api.cohere.ai/v1/rerank".to_string(),
            max_chunks_per_request: 1000,
        }
    }

    pub fn with_custom_url(mut self, url: String) -> Self {
        self.api_url = url;
        self
    }

    pub fn with_max_chunks(mut self, max_chunks: usize) -> Self {
        self.max_chunks_per_request = max_chunks;
        self
    }
}

#[derive(Serialize)]
struct CohereRerankRequest<'a> {
    model: &'a str,
    query: &'a str,
    documents: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_n: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    return_documents: Option<bool>,
}

#[derive(Deserialize)]
struct CohereRerankResultItem {
    index: usize,
    relevance_score: f32,
}

#[derive(Deserialize)]
struct CohereRerankResponse {
    results: Vec<CohereRerankResultItem>,
}

#[async_trait]
impl Reranker for CohereReranker {
    #[instrument(skip(self, results))]
    async fn rerank(&self, query: &str, results: Vec<Citation>) -> Result<Vec<Citation>> {
        if results.is_empty() {
            return Ok(results);
        }

        // 分批处理如果结果太多
        let mut reranked_results = Vec::new();
        for chunk in results.chunks(self.max_chunks_per_request) {
            let chunk_results = self.rerank_batch(query, chunk.to_vec()).await?;
            reranked_results.extend(chunk_results);
        }

        // 如果有多个批次，需要再次排序
        if results.len() > self.max_chunks_per_request {
            reranked_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }

        Ok(reranked_results)
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> Result<()> {
        // 简单的健康检查 - 尝试发送一个小的测试请求
        let test_request = CohereRerankRequest {
            model: &self.model,
            query: "test",
            documents: vec!["test document"],
            top_n: Some(1),
            return_documents: Some(false),
        };

        let response = self.client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .header("Cohere-Version", "2022-12-06")
            .json(&test_request)
            .send()
            .await
            .map_err(|e| KbError::Network {
                operation: "cohere_health_check".to_string(),
                message: e.to_string(),
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(KbError::ServiceUnavailable {
                service: "cohere".to_string(),
                retry_after: Some(std::time::Duration::from_secs(60)),
            })
        }
    }
}

impl CohereReranker {
    async fn rerank_batch(&self, query: &str, results: Vec<Citation>) -> Result<Vec<Citation>> {
        let documents: Vec<&str> = results.iter().map(|c| c.snippet.as_str()).collect();

        let request = CohereRerankRequest {
            model: &self.model,
            query,
            documents,
            top_n: None,
            return_documents: Some(false),
        };

        let response = self.client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .header("Cohere-Version", "2022-12-06")
            .json(&request)
            .send()
            .await
            .map_err(|e| KbError::Network {
                operation: "cohere_rerank".to_string(),
                message: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let _error_text = response.text().await.unwrap_or_default();
            return Err(KbError::ServiceUnavailable {
                service: format!("cohere ({})", status),
                retry_after: if status.as_u16() == 429 {
                    Some(std::time::Duration::from_secs(60))
                } else {
                    None
                },
            });
        }

        let cohere_response: CohereRerankResponse = response.json().await.map_err(|e| {
            KbError::Serialization {
                format: "json".to_string(),
                message: e.to_string(),
            }
        })?;

        // 重新排序结果
        let mut indexed_scores: Vec<(usize, f32)> = cohere_response
            .results
            .into_iter()
            .map(|item| (item.index, item.relevance_score))
            .collect();

        // 按 relevance_score 降序排序
        indexed_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 构建重排后的结果
        let mut reranked = Vec::with_capacity(results.len());
        for (original_index, relevance_score) in indexed_scores {
            if let Some(mut citation) = results.get(original_index).cloned() {
                citation.score = relevance_score;
                reranked.push(citation);
            }
        }

        tracing::info!(
            query = %query,
            original_count = results.len(),
            reranked_count = reranked.len(),
            "Cohere reranking completed"
        );

        Ok(reranked)
    }
}

impl SemanticReranker {
    pub fn new(
        embed_model: std::sync::Arc<dyn kb_llm::EmbedModel>,
        similarity_threshold: f32,
        boost_factor: f32,
    ) -> Self {
        Self {
            name: "semantic".to_string(),
            embed_model,
            similarity_threshold,
            boost_factor,
        }
    }

    async fn calculate_semantic_similarity(&self, query: &str, document: &str) -> Result<f32> {
        let query_texts = vec![query.to_string()];
        let doc_texts = vec![document.to_string()];

        let query_embeddings = self.embed_model.embed(&query_texts).await.map_err(|e| {
            kb_error::KbError::LlmService {
                provider: "embedding".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            }
        })?;

        let doc_embeddings = self.embed_model.embed(&doc_texts).await.map_err(|e| {
            kb_error::KbError::LlmService {
                provider: "embedding".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            }
        })?;

        if let (Some(query_emb), Some(doc_emb)) = (query_embeddings.first(), doc_embeddings.first()) {
            Ok(cosine_similarity(query_emb, doc_emb))
        } else {
            Ok(0.0)
        }
    }
}

#[async_trait]
impl Reranker for SemanticReranker {
    #[instrument(skip(self, results))]
    async fn rerank(&self, query: &str, mut results: Vec<Citation>) -> Result<Vec<Citation>> {
        for citation in &mut results {
            let semantic_score = self.calculate_semantic_similarity(query, &citation.snippet).await?;

            if semantic_score >= self.similarity_threshold {
                citation.score = citation.score * (1.0 + semantic_score * self.boost_factor);
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        tracing::info!(
            query = %query,
            results_count = results.len(),
            "Semantic reranking completed"
        );

        Ok(results)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl LengthReranker {
    pub fn new(optimal_length: usize, length_penalty_factor: f32) -> Self {
        Self {
            name: "length".to_string(),
            optimal_length,
            length_penalty_factor,
        }
    }

    fn calculate_length_score(&self, text_length: usize) -> f32 {
        let deviation = (text_length as f32 - self.optimal_length as f32).abs();
        let normalized_deviation = deviation / self.optimal_length as f32;

        // 长度越接近最优长度，奖励越大
        let length_bonus = (-normalized_deviation * self.length_penalty_factor).exp();
        length_bonus
    }
}

#[async_trait]
impl Reranker for LengthReranker {
    #[instrument(skip(self, results))]
    async fn rerank(&self, query: &str, mut results: Vec<Citation>) -> Result<Vec<Citation>> {
        for citation in &mut results {
            let length_score = self.calculate_length_score(citation.snippet.len());
            citation.score = citation.score * length_score;
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        tracing::info!(
            query = %query,
            results_count = results.len(),
            optimal_length = self.optimal_length,
            "Length reranking completed"
        );

        Ok(results)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl DiversityReranker {
    pub fn new(
        embed_model: std::sync::Arc<dyn kb_llm::EmbedModel>,
        lambda: f32,
        similarity_threshold: f32,
    ) -> Self {
        Self {
            name: "diversity".to_string(),
            embed_model,
            lambda,
            similarity_threshold,
        }
    }

    // TODO: 修复编译问题后重新启用
    /*
    /// 实现最大边际相关性(MMR)算法
    async fn maximal_marginal_relevance(&self, query: &str, documents: Vec<Citation>) -> Result<Vec<Citation>> {
        // 简化实现，暂时直接返回原结果
        Ok(documents)
    }
    */
}

// TODO: 修复编译问题后重新启用
/*
#[async_trait]
impl Reranker for DiversityReranker {
    #[instrument(skip(self, results))]
    async fn rerank(&self, query: &str, results: Vec<Citation>) -> Result<Vec<Citation>> {
        if results.len() <= 1 {
            return Ok(results);
        }

        let diversified = self.maximal_marginal_relevance(query, results).await?;

        tracing::info!(
            query = %query,
            results_count = diversified.len(),
            lambda = self.lambda,
            "Diversity reranking completed"
        );

        Ok(diversified)
    }

    fn name(&self) -> &str {
        &self.name
    }
}
*/

/// 余弦相似度计算函数
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for i in 0..a.len() {
        dot_product += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot_product / (norm_a.sqrt() * norm_b.sqrt())
}

/// 组合重排器 - 可以串联多个重排器
pub struct CompositeReranker {
    name: String,
    rerankers: Vec<Box<dyn Reranker>>,
}

impl CompositeReranker {
    pub fn new() -> Self {
        Self {
            name: "composite".to_string(),
            rerankers: Vec::new(),
        }
    }

    pub fn add_reranker(mut self, reranker: Box<dyn Reranker>) -> Self {
        self.rerankers.push(reranker);
        self
    }
}

#[async_trait]
impl Reranker for CompositeReranker {
    #[instrument(skip(self, results))]
    async fn rerank(&self, query: &str, mut results: Vec<Citation>) -> Result<Vec<Citation>> {
        for reranker in &self.rerankers {
            results = reranker.rerank(query, results).await?;
        }

        tracing::info!(
            query = %query,
            rerankers_count = self.rerankers.len(),
            final_results_count = results.len(),
            "Composite reranking completed"
        );

        Ok(results)
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> Result<()> {
        let mut errors = Vec::new();

        for reranker in &self.rerankers {
            if let Err(e) = reranker.health_check().await {
                errors.push(format!("{}: {}", reranker.name(), e));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(KbError::ServiceUnavailable {
                service: format!("composite_reranker ({})", errors.join(", ")),
                retry_after: Some(std::time::Duration::from_secs(30)),
            })
        }
    }
}

/// 重排器工厂
pub struct RerankerFactory;

impl RerankerFactory {
    /// 根据配置创建重排器
    pub fn create_from_env() -> Result<Option<Box<dyn Reranker>>> {
        // 检查 Cohere API Key
        if let Ok(api_key) = std::env::var("COHERE_API_KEY") {
            let model = std::env::var("COHERE_RERANK_MODEL").ok();
            let cohere = CohereReranker::new(api_key, model);

            // 可选择性添加关键词重排器作为预处理
            let use_keyword_boost = std::env::var("RERANK_USE_KEYWORD_BOOST")
                .unwrap_or_else(|_| "true".to_string())
                .parse::<bool>()
                .unwrap_or(true);

            if use_keyword_boost {
                let composite = CompositeReranker::new()
                    .add_reranker(Box::new(KeywordReranker::new(false, 0.1)))
                    .add_reranker(Box::new(cohere));
                return Ok(Some(Box::new(composite)));
            } else {
                return Ok(Some(Box::new(cohere)));
            }
        }

        // 回退到关键词重排器
        let use_keyword_fallback = std::env::var("RERANK_USE_KEYWORD_FALLBACK")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        if use_keyword_fallback {
            Ok(Some(Box::new(KeywordReranker::new(false, 0.2))))
        } else {
            Ok(None)
        }
    }

    /// 创建关键词重排器
    pub fn keyword_reranker(case_sensitive: bool, boost_factor: f32) -> Box<dyn Reranker> {
        Box::new(KeywordReranker::new(case_sensitive, boost_factor))
    }

    /// 创建 Cohere 重排器
    pub fn cohere_reranker(api_key: String, model: Option<String>) -> Box<dyn Reranker> {
        Box::new(CohereReranker::new(api_key, model))
    }

    /// 创建语义相似度重排器
    pub fn semantic_reranker(
        embed_model: std::sync::Arc<dyn kb_llm::EmbedModel>,
        similarity_threshold: f32,
        boost_factor: f32,
    ) -> Box<dyn Reranker> {
        Box::new(SemanticReranker::new(embed_model, similarity_threshold, boost_factor))
    }

    /// 创建长度重排器
    pub fn length_reranker(optimal_length: usize, penalty_factor: f32) -> Box<dyn Reranker> {
        Box::new(LengthReranker::new(optimal_length, penalty_factor))
    }

    /// 创建多样性重排器（暂时禁用由于编译问题）
    pub fn diversity_reranker(
        _embed_model: std::sync::Arc<dyn kb_llm::EmbedModel>,
        _lambda: f32,
        _similarity_threshold: f32,
    ) -> Box<dyn Reranker> {
        // 暂时返回关键词重排器作为替代
        Box::new(KeywordReranker::new(false, 0.2))
    }

    /// 从环境变量创建高级重排器链
    pub fn create_advanced_reranker_chain() -> Result<Option<Box<dyn Reranker>>> {
        let mut composite = CompositeReranker::new();
        let mut has_rerankers = false;

        // 1. 长度重排器
        if let Ok(optimal_length) = std::env::var("RERANK_OPTIMAL_LENGTH") {
            if let Ok(length) = optimal_length.parse::<usize>() {
                let penalty = std::env::var("RERANK_LENGTH_PENALTY")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.1);
                composite = composite.add_reranker(Self::length_reranker(length, penalty));
                has_rerankers = true;
            }
        }

        // 2. 关键词重排器（如果启用）
        let use_keyword = std::env::var("RERANK_USE_KEYWORD")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        if use_keyword {
            let boost = std::env::var("RERANK_KEYWORD_BOOST")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.2);
            composite = composite.add_reranker(Self::keyword_reranker(false, boost));
            has_rerankers = true;
        }

        // 3. Cohere 重排器（如果有 API Key）
        if let Ok(api_key) = std::env::var("COHERE_API_KEY") {
            let model = std::env::var("COHERE_RERANK_MODEL").ok();
            composite = composite.add_reranker(Self::cohere_reranker(api_key, model));
            has_rerankers = true;
        }

        if has_rerankers {
            Ok(Some(Box::new(composite)))
        } else {
            Ok(None)
        }
    }
}