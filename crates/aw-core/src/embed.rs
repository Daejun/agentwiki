//! 로컬 임베딩 (D9/D18).
//!
//! 하이브리드 검색의 벡터 절반을 담당한다. 실제 모델은 fastembed-rs(다국어
//! 소형모델, 예: multilingual-e5-small)로 붙이되, 모델 다운로드가 네트워크를
//! 요구하므로 `embeddings` feature로 게이트한다. feature가 꺼져 있으면
//! `NoopEmbedder`가 빈 벡터를 돌려주고 검색은 FTS5 단독으로 동작한다.

/// 텍스트를 임베딩 벡터로 변환하는 추상 인터페이스.
pub trait Embedder: Send + Sync {
    /// 임베딩 차원. 0이면 "임베딩 비활성"을 뜻한다.
    fn dim(&self) -> usize;
    /// 여러 텍스트를 한 번에 임베딩.
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>>;
    fn enabled(&self) -> bool {
        self.dim() > 0
    }
}

/// 임베딩 비활성 구현(기본). FTS5 단독 검색 경로에서 쓰인다.
pub struct NoopEmbedder;

impl Embedder for NoopEmbedder {
    fn dim(&self) -> usize {
        0
    }
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.iter().map(|_| Vec::new()).collect()
    }
}

#[cfg(feature = "embeddings")]
pub use fastembed_impl::FastEmbedder;

#[cfg(feature = "embeddings")]
mod fastembed_impl {
    // fastembed-rs 연동 자리. `embeddings` feature가 켜졌을 때만 컴파일된다.
    // 의도적으로 의존성 추가는 M1 범위 밖(잠긴 환경 빌드 보호)으로 미뤄두고,
    // 트레이트 계약만 고정한다. 실제 구현 시 fastembed::TextEmbedding을 감싼다.
    use super::Embedder;

    pub struct FastEmbedder {
        dim: usize,
    }

    impl FastEmbedder {
        pub fn new(dim: usize) -> Self {
            Self { dim }
        }
    }

    impl Embedder for FastEmbedder {
        fn dim(&self) -> usize {
            self.dim
        }
        fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
            // TODO(M4): fastembed::TextEmbedding::embed 로 교체.
            texts.iter().map(|_| vec![0.0; self.dim]).collect()
        }
    }
}

/// 코사인 유사도. 벡터 검색 랭킹에 사용.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}
