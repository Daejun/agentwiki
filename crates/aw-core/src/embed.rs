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

/// 임베딩 비활성 구현. FTS5 단독 검색 경로에서 쓰인다.
pub struct NoopEmbedder;

impl Embedder for NoopEmbedder {
    fn dim(&self) -> usize {
        0
    }
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.iter().map(|_| Vec::new()).collect()
    }
}

/// 경량 로컬 임베더 — 문자 트라이그램 feature hashing (D9, 기본 경로).
///
/// 외부 모델·네트워크 없이 동작하는 결정론적 임베딩이다. 텍스트를 문자
/// 트라이그램으로 쪼개(한국어에 잘 맞음, FTS5 trigram과 같은 논리) 각 트라이그램을
/// 고정 차원 버킷에 해시·누적하고 L2 정규화한다. 코사인 유사도가 어휘적 의미
/// 중첩을 포착해 FTS5와의 RRF 융합에 실제 벡터 신호를 더한다.
///
/// 학습된 다국어 모델(fastembed)만큼 풍부하진 않지만, 의존성 0으로 하이브리드
/// 검색 경로를 실제로 활성화한다. 고품질이 필요하면 `embeddings` feature의
/// `FastEmbedder`로 교체한다(D18).
pub struct HashEmbedder {
    dim: usize,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self { dim: 256 }
    }
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(16) }
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        let lower = text.to_lowercase();
        let chars: Vec<char> = lower.chars().filter(|c| !c.is_whitespace()).collect();
        if chars.len() < 3 {
            // 짧은 텍스트는 개별 문자도 신호로 사용.
            for &c in &chars {
                let h = fnv1a(&[c]);
                bump(&mut v, h, self.dim);
            }
        } else {
            for w in chars.windows(3) {
                let h = fnv1a(w);
                bump(&mut v, h, self.dim);
            }
        }
        l2_normalize(&mut v);
        v
    }
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.iter().map(|t| self.embed_one(t)).collect()
    }
}

/// FNV-1a 해시(문자 시퀀스).
fn fnv1a(chars: &[char]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &c in chars {
        for b in (c as u32).to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// 버킷에 부호 해싱으로 누적(충돌 편향 완화).
fn bump(v: &mut [f32], h: u64, dim: usize) {
    let idx = (h % dim as u64) as usize;
    let sign = if (h >> 63) & 1 == 1 { 1.0 } else { -1.0 };
    v[idx] += sign;
}

fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embedder_is_deterministic_and_normalized() {
        let e = HashEmbedder::default();
        let a = &e.embed(&["mmap_write_lock 규약".into()])[0];
        let b = &e.embed(&["mmap_write_lock 규약".into()])[0];
        assert_eq!(a, b);
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn similar_text_scores_higher_than_unrelated() {
        let e = HashEmbedder::default();
        let q = &e.embed(&["mmap 락 규약".into()])[0];
        let near = &e.embed(&["mmap 락 보유 규약".into()])[0];
        let far = &e.embed(&["네트워크 소켓 버퍼 할당".into()])[0];
        let s_near = cosine(q, near);
        let s_far = cosine(q, far);
        assert!(s_near > s_far, "near={s_near} should beat far={s_far}");
    }

    #[test]
    fn enabled_reflects_dim() {
        assert!(!NoopEmbedder.enabled());
        assert!(HashEmbedder::default().enabled());
    }
}
