//! Pentanomial statistics and the sequential probability ratio test.
//!
//! A game pair scores 0, 1/4, 1/2, 3/4 or 1 for the tested engine. The test
//! uses the normal approximation of the generalized SPRT on those pair
//! scores (as popularized by Fishtest), with bounds in logistic Elo.

/// Counts of game pairs by total score in half points (0 to 4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pentanomial {
    pub counts: [u64; 5],
}

/// Expected score for an Elo difference.
fn score_of(elo: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-elo / 400.0))
}

/// Elo difference for an expected score.
fn elo_of(score: f64) -> f64 {
    let s = score.clamp(1e-6, 1.0 - 1e-6);
    -400.0 * (1.0 / s - 1.0).log10()
}

impl Pentanomial {
    /// Record a pair worth `half_points` (0 to 4).
    pub fn add(&mut self, half_points: usize) {
        self.counts[half_points] += 1;
    }

    /// Pairs, mean pair score and its per-pair variance. A small prior keeps
    /// the variance positive for tiny samples.
    fn stats(&self) -> (f64, f64, f64) {
        let counts: Vec<f64> = self.counts.iter().map(|&c| c as f64 + 1e-3).collect();
        let n: f64 = counts.iter().sum();
        let mean = counts.iter().enumerate().map(|(i, c)| c * i as f64 / 4.0).sum::<f64>() / n;
        let var = counts
            .iter()
            .enumerate()
            .map(|(i, c)| c * (i as f64 / 4.0 - mean).powi(2))
            .sum::<f64>()
            / n;
        (n, mean, var)
    }

    /// Number of pairs recorded.
    pub fn pairs(&self) -> u64 {
        self.counts.iter().sum()
    }

    /// Elo estimate and the half-width of its 95% interval.
    pub fn elo(&self) -> (f64, f64) {
        if self.pairs() == 0 {
            return (0.0, 0.0);
        }
        let (n, mean, var) = self.stats();
        let margin = 1.96 * (var / n).sqrt();
        let elo = elo_of(mean);
        let error = (elo_of(mean + margin) - elo_of(mean - margin)) / 2.0;
        (elo, error)
    }
}

/// A test of H0: Elo = `elo0` against H1: Elo = `elo1`.
pub struct Sprt {
    elo0: f64,
    elo1: f64,
    alpha: f64,
    beta: f64,
}

impl Sprt {
    pub fn new(elo0: f64, elo1: f64, alpha: f64, beta: f64) -> Sprt {
        Sprt {
            elo0,
            elo1,
            alpha,
            beta,
        }
    }

    /// Accept H0 at or below this log-likelihood ratio.
    pub fn lower(&self) -> f64 {
        (self.beta / (1.0 - self.alpha)).ln()
    }

    /// Accept H1 at or above this log-likelihood ratio.
    pub fn upper(&self) -> f64 {
        ((1.0 - self.beta) / self.alpha).ln()
    }

    /// Log-likelihood ratio of the results so far.
    pub fn llr(&self, results: &Pentanomial) -> f64 {
        if results.pairs() == 0 {
            return 0.0;
        }
        let (n, mean, var) = results.stats();
        let (s0, s1) = (score_of(self.elo0), score_of(self.elo1));
        n * (s1 - s0) * (2.0 * mean - s0 - s1) / (2.0 * var)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_results_give_zero_elo() {
        let mut p = Pentanomial::default();
        for s in [0, 1, 2, 2, 2, 3, 4] {
            p.add(s);
        }
        assert!(p.elo().0.abs() < 1e-6);
    }

    #[test]
    fn strong_results_accept_h1() {
        let sprt = Sprt::new(0.0, 5.0, 0.05, 0.05);
        let mut p = Pentanomial::default();
        for i in 0..4000 {
            // 60% pair score on average: clearly stronger.
            p.add([2, 3, 2, 3, 2, 3, 4, 1, 2, 2][i % 10]);
        }
        assert!(sprt.llr(&p) > sprt.upper());
        assert!(p.elo().0 > 30.0);
        let mut weak = Pentanomial::default();
        for i in 0..4000 {
            weak.add([2, 1, 2, 1, 2, 1, 0, 3, 2, 2][i % 10]);
        }
        assert!(sprt.llr(&weak) < sprt.lower());
        assert!((sprt.lower() + 2.944).abs() < 0.01 && (sprt.upper() - 2.944).abs() < 0.01);
    }
}
