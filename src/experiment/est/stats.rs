/// Linear-interpolation percentile over [0.0, 1.0]. Returns 0.0 for an empty slice.
pub(crate) fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    if values.len() == 1 {
        return values[0];
    }

    let quantile = quantile.clamp(0.0, 1.0);
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));

    let rank = quantile * (sorted.len() as f64 - 1.0);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;

    if lower == upper {
        sorted[lower]
    } else {
        let fraction = rank - lower as f64;
        sorted[lower] + fraction * (sorted[upper] - sorted[lower])
    }
}

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn percentile_uses_linear_interpolation() {
        let values = [10.0, 20.0, 30.0, 40.0];

        assert!((percentile(&values, 0.25) - 17.5).abs() < 1e-9);
        assert!((percentile(&values, 0.50) - 25.0).abs() < 1e-9);
        assert!((percentile(&values, 0.75) - 32.5).abs() < 1e-9);
        assert!((percentile(&values, 0.90) - 37.0).abs() < 1e-9);
    }

    #[test]
    fn percentile_returns_zero_for_empty_input() {
        assert_eq!(percentile(&[], 0.90), 0.0);
    }
}
