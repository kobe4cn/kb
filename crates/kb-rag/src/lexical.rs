use async_trait::async_trait;
use kb_core::{Citation, QueryRequest, QueryResponse};
use kb_error::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

use crate::engine::{BaseRagEngine, EngineStats, HealthStatus, RagEngine, RagMeta};

/// 词汇检索引擎 - 基于关键词匹配和 TF-IDF 评分
pub struct LexicalRagEngine {
    base: BaseRagEngine,
    index: Arc<RwLock<LexicalIndex>>,
    config: LexicalConfig,
}

/// 词汇检索配置
#[derive(Debug, Clone)]
pub struct LexicalConfig {
    /// 是否区分大小写
    pub case_sensitive: bool,
    /// 停用词列表
    pub stop_words: HashSet<String>,
    /// 最小词长
    pub min_word_length: usize,
    /// 最大查询词数
    pub max_query_terms: usize,
    /// 是否启用词干提取
    pub enable_stemming: bool,
    /// TF-IDF 权重
    pub tfidf_weight: f32,
    /// 关键词匹配权重
    pub keyword_weight: f32,
}

impl Default for LexicalConfig {
    fn default() -> Self {
        let mut stop_words = HashSet::new();
        // 中文停用词
        for word in &[
            "的", "了", "在", "是", "我", "有", "和", "就", "不", "人", "都", "一", "一个", "上",
            "也", "很", "到", "说", "要", "去", "你", "会", "着", "没有", "看", "好", "自己", "这",
        ] {
            stop_words.insert(word.to_string());
        }
        // 英文停用词
        for word in &[
            "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with",
            "by", "this", "that", "is", "are", "was", "were", "be", "been", "have", "has", "had",
            "do", "does", "did", "will", "would", "could", "should",
        ] {
            stop_words.insert(word.to_string());
        }

        Self {
            case_sensitive: false,
            stop_words,
            min_word_length: 2,
            max_query_terms: 20,
            enable_stemming: false,
            tfidf_weight: 0.7,
            keyword_weight: 0.3,
        }
    }
}

/// 词汇索引结构
#[derive(Debug, Default)]
pub struct LexicalIndex {
    /// 文档 -> 词频统计
    document_term_freq: HashMap<String, HashMap<String, u32>>,
    /// 词 -> 包含该词的文档列表
    inverted_index: HashMap<String, HashSet<String>>,
    /// 文档 -> 文档内容
    documents: HashMap<String, DocumentInfo>,
    /// 文档总数
    total_documents: u32,
}

/// 文档信息
#[derive(Debug, Clone)]
pub struct DocumentInfo {
    pub document_id: String,
    pub chunk_id: String,
    pub content: String,
    pub page: Option<i32>,
    pub meta: Option<RagMeta>,
    pub word_count: u32,
}

/// 搜索结果
#[derive(Debug, Clone)]
pub struct LexicalSearchResult {
    pub document_id: String,
    pub chunk_id: String,
    pub score: f32,
    pub matched_terms: Vec<String>,
    pub snippet: String,
}

impl LexicalRagEngine {
    pub fn new(base: BaseRagEngine, config: LexicalConfig) -> Self {
        Self {
            base,
            index: Arc::new(RwLock::new(LexicalIndex::default())),
            config,
        }
    }

    /// 添加文档到词汇索引
    #[instrument(skip(self, text))]
    pub async fn index_document(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        let chunk_records = self
            .base
            .chunk_document(document_id, text, page, meta.clone());
        let mut index = self.index.write().await;

        for chunk in chunk_records.iter() {
            let tokens = self.tokenize(&chunk.text);
            let term_freq = self.calculate_term_frequency(&tokens);

            // 更新倒排索引
            for term in term_freq.keys() {
                index
                    .inverted_index
                    .entry(term.clone())
                    .or_insert_with(HashSet::new)
                    .insert(chunk.chunk_id.clone());
            }

            // 存储文档信息
            let doc_info = DocumentInfo {
                document_id: chunk.document_id.clone(),
                chunk_id: chunk.chunk_id.clone(),
                content: chunk.text.clone(),
                page: chunk.page,
                meta: Some(chunk.as_meta()),
                word_count: tokens.len() as u32,
            };

            index.documents.insert(chunk.chunk_id.clone(), doc_info);
            index
                .document_term_freq
                .insert(chunk.chunk_id.clone(), term_freq);
        }

        index.total_documents = index.documents.len() as u32;
        debug!(
            document_id,
            chunks = chunk_records.len(),
            "文档已添加到词汇索引"
        );

        Ok(())
    }

    /// 执行词汇搜索
    #[instrument(skip(self))]
    pub async fn search(
        &self,
        query: &str,
        max_results: Option<usize>,
    ) -> Result<Vec<LexicalSearchResult>> {
        let query_tokens = self.tokenize(query);
        if query_tokens.is_empty() {
            return Ok(vec![]);
        }

        let index = self.index.read().await;
        let mut candidates = HashSet::new();

        // 收集候选文档
        for token in &query_tokens {
            if let Some(docs) = index.inverted_index.get(token) {
                candidates.extend(docs.iter().cloned());
            }
        }

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        // 计算每个候选文档的分数
        let mut results = Vec::new();
        let candidates_count = candidates.len();
        for chunk_id in &candidates {
            if let Some(doc_info) = index.documents.get(chunk_id) {
                let score = self.calculate_score(&query_tokens, chunk_id, &index);
                let matched_terms = self.find_matched_terms(&query_tokens, chunk_id, &index);

                if score > 0.0 {
                    results.push(LexicalSearchResult {
                        document_id: doc_info.document_id.clone(),
                        chunk_id: chunk_id.clone(),
                        score,
                        matched_terms,
                        snippet: self.extract_snippet(&doc_info.content, &query_tokens),
                    });
                }
            }
        }

        // 按分数排序
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 限制结果数量
        let max_results = max_results.unwrap_or(10);
        results.truncate(max_results);

        debug!(
            query = %query,
            candidates = candidates_count,
            results = results.len(),
            "词汇搜索完成"
        );

        Ok(results)
    }

    /// 分词和预处理
    fn tokenize(&self, text: &str) -> Vec<String> {
        let text = if self.config.case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };

        // 简单的分词：按空白字符和标点分割
        let chinese_punct = [
            '，', '。', '！', '？', '；', '：', '"', '"', '\'', '\'', '（', '）', '【', '】', '《',
            '》',
        ];
        let tokens: Vec<String> = text
            .split(|c: char| {
                c.is_whitespace() || c.is_ascii_punctuation() || chinese_punct.contains(&c)
            })
            .filter_map(|word| {
                let trimmed = word.trim();
                if trimmed.len() >= self.config.min_word_length
                    && !self.config.stop_words.contains(trimmed)
                {
                    Some(trimmed.to_string())
                } else {
                    None
                }
            })
            .take(self.config.max_query_terms)
            .collect();

        tokens
    }

    /// 计算词频
    fn calculate_term_frequency(&self, tokens: &[String]) -> HashMap<String, u32> {
        let mut term_freq = HashMap::new();
        for token in tokens {
            *term_freq.entry(token.clone()).or_insert(0) += 1;
        }
        term_freq
    }

    /// 计算 TF-IDF 分数
    fn calculate_score(
        &self,
        query_tokens: &[String],
        chunk_id: &str,
        index: &LexicalIndex,
    ) -> f32 {
        if let Some(doc_term_freq) = index.document_term_freq.get(chunk_id) {
            let mut score = 0.0;
            let mut matched_terms = 0;

            for query_term in query_tokens {
                if let Some(&term_freq) = doc_term_freq.get(query_term) {
                    matched_terms += 1;

                    // TF 分数（词频）
                    let tf = term_freq as f32;

                    // IDF 分数（逆文档频率）
                    let doc_freq = index
                        .inverted_index
                        .get(query_term)
                        .map(|docs| docs.len())
                        .unwrap_or(1) as f32;
                    let idf = (index.total_documents as f32 / doc_freq).ln();

                    // TF-IDF 分数
                    let tfidf_score = tf * idf;
                    score += tfidf_score * self.config.tfidf_weight;
                }
            }

            // 关键词匹配度奖励
            if matched_terms > 0 {
                let keyword_score =
                    (matched_terms as f32 / query_tokens.len() as f32) * self.config.keyword_weight;
                score += keyword_score;
            }

            score
        } else {
            0.0
        }
    }

    /// 找到匹配的词汇
    fn find_matched_terms(
        &self,
        query_tokens: &[String],
        chunk_id: &str,
        index: &LexicalIndex,
    ) -> Vec<String> {
        if let Some(doc_term_freq) = index.document_term_freq.get(chunk_id) {
            query_tokens
                .iter()
                .filter(|term| doc_term_freq.contains_key(*term))
                .cloned()
                .collect()
        } else {
            vec![]
        }
    }

    /// 提取上下文片段
    fn extract_snippet(&self, content: &str, query_tokens: &[String]) -> String {
        let max_snippet_length = 300;

        // 如果内容较短，直接返回
        if content.len() <= max_snippet_length {
            return content.to_string();
        }

        // 寻找第一个匹配词的位置
        let content_lower = content.to_lowercase();
        let mut best_position = 0;

        for token in query_tokens {
            if let Some(pos) = content_lower.find(&token.to_lowercase()) {
                best_position = pos;
                break;
            }
        }

        // 以匹配位置为中心提取片段
        let start = best_position.saturating_sub(max_snippet_length / 2);
        let end = (start + max_snippet_length).min(content.len());

        let mut snippet = content[start..end].to_string();

        // 添加省略号
        if start > 0 {
            snippet = format!("...{}", snippet);
        }
        if end < content.len() {
            snippet = format!("{}...", snippet);
        }

        snippet
    }

    /// 获取索引统计信息
    pub async fn get_index_stats(&self) -> Result<LexicalIndexStats> {
        let index = self.index.read().await;
        Ok(LexicalIndexStats {
            total_documents: index.total_documents,
            total_terms: index.inverted_index.len() as u32,
            average_document_length: if index.total_documents > 0 {
                index
                    .documents
                    .values()
                    .map(|doc| doc.word_count)
                    .sum::<u32>() as f32
                    / index.total_documents as f32
            } else {
                0.0
            },
        })
    }

    /// 清空索引
    pub async fn clear_index(&self) -> Result<()> {
        let mut index = self.index.write().await;
        index.document_term_freq.clear();
        index.inverted_index.clear();
        index.documents.clear();
        index.total_documents = 0;
        debug!("词汇索引已清空");
        Ok(())
    }
}

/// 词汇索引统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LexicalIndexStats {
    pub total_documents: u32,
    pub total_terms: u32,
    pub average_document_length: f32,
}

#[async_trait]
impl RagEngine for LexicalRagEngine {
    #[instrument(skip(self, req))]
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        let start_time = std::time::Instant::now();

        // 执行词汇搜索
        let search_results = self
            .search(&req.query, req.top_k.map(|k| k as usize))
            .await?;

        if search_results.is_empty() {
            return Ok(QueryResponse {
                answer: "没有找到相关的文档内容".to_string(),
                citations: vec![],
                contexts: vec![],
                mode: "lexical".to_string(),
                latency_ms: start_time.elapsed().as_millis() as i64,
            });
        }

        // 转换为引用格式
        let citations: Vec<Citation> = search_results
            .iter()
            .enumerate()
            .map(|(_idx, result)| Citation {
                document_id: result.document_id.clone(),
                chunk_id: result.chunk_id.clone(),
                page: None, // 从文档信息中获取
                score: result.score,
                snippet: result.snippet.clone(),
            })
            .collect();

        // 格式化上下文
        let context = self.base.format_context(&citations);
        let contexts: Vec<String> = citations.iter().map(|c| c.snippet.clone()).collect();

        // 生成回答
        let answer = self.base.generate_answer(&context, &req.query).await?;

        Ok(QueryResponse {
            answer,
            citations,
            contexts,
            mode: "lexical".to_string(),
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
        self.index_document(document_id, text, page, meta).await
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        let stats = self.get_index_stats().await?;
        if stats.total_documents > 0 {
            Ok(HealthStatus::Healthy)
        } else {
            Ok(HealthStatus::Degraded {
                reason: "No documents in lexical index".to_string(),
            })
        }
    }

    async fn stats(&self) -> Result<EngineStats> {
        let index_stats = self.get_index_stats().await?;
        Ok(EngineStats {
            total_documents: index_stats.total_documents as u64,
            total_chunks: index_stats.total_documents as u64, // 在词汇索引中，文档和chunk是一对一的
            index_size_bytes: 0,                              // 可以计算内存使用
            last_updated: Some(chrono::Utc::now()),
            query_count: 0, // 可以添加计数器
            average_query_latency_ms: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::RagEngineConfig;

    // 简单的 Mock 实现用于测试
    struct MockChatModel;
    struct MockEmbedModel;

    #[async_trait]
    impl kb_llm::ChatModel for MockChatModel {
        async fn chat(
            &self,
            _system: &str,
            _context: &str,
            _query: &str,
        ) -> kb_llm::Result<String> {
            Ok("Mock response".to_string())
        }
    }

    #[async_trait]
    impl kb_llm::EmbedModel for MockEmbedModel {
        async fn embed(&self, _texts: &[String]) -> kb_llm::Result<Vec<Vec<f32>>> {
            Ok(vec![vec![0.1, 0.2, 0.3]; _texts.len()])
        }
    }

    fn create_test_engine() -> LexicalRagEngine {
        let chat_model = Arc::new(MockChatModel);
        let embed_model = Arc::new(MockEmbedModel);
        let base = BaseRagEngine::new(chat_model, embed_model, RagEngineConfig::default());
        LexicalRagEngine::new(base, LexicalConfig::default())
    }

    #[tokio::test]
    async fn test_tokenize() {
        let engine = create_test_engine();
        let tokens = engine.tokenize("Hello world! This is a test. 这是一个测试。");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        assert!(tokens.contains(&"这是一个测试".to_string()));
        // 停用词应该被过滤掉
        assert!(!tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[tokio::test]
    async fn test_add_and_search() {
        let engine = create_test_engine();

        // 添加文档
        engine
            .add_document_text("doc1", "Rust is a systems programming language", None)
            .await
            .unwrap();
        engine
            .add_document_text("doc2", "Python is a high-level programming language", None)
            .await
            .unwrap();

        // 搜索
        let results = engine
            .search("programming language", Some(10))
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].score > 0.0);
        assert!(results[0]
            .matched_terms
            .contains(&"programming".to_string()));
    }
}
