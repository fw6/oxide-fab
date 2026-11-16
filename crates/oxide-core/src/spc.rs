//! Statistical process control helpers.
//!
//! The simulator measures one parameter per metrology wafer. The chart in the
//! dashboard plots the samples the time series store holds, with the route's
//! specification limits and the process's own 3-sigma limits.

/// Mean and population standard deviation of `values`.
pub fn mean_sigma(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    (mean, variance.sqrt())
}

/// Control limits: `mean ± 3 sigma`. An empty sample yields zeros.
pub fn control_limits(values: &[f64]) -> (f64, f64, f64) {
    let (mean, sigma) = mean_sigma(values);
    (mean, mean + 3.0 * sigma, mean - 3.0 * sigma)
}

/// `true` when `value` sits outside `[lcl, ucl]`.
pub fn out_of_control(value: f64, lcl: f64, ucl: f64) -> bool {
    value < lcl || value > ucl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_three_sigma_around_the_mean() {
        let values = [10.0, 10.0, 10.0, 10.0];
        let (mean, ucl, lcl) = control_limits(&values);
        assert!((mean - 10.0).abs() < 1e-9);
        assert!((ucl - 10.0).abs() < 1e-9);
        assert!((lcl - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_wide_scatter_marks_values_out_of_control() {
        let values = [9.0, 10.0, 11.0];
        let (_, ucl, lcl) = control_limits(&values);
        // mean 10, sigma 0.8165, so the 3-sigma band is 7.55 … 12.45.
        assert!(out_of_control(12.6, lcl, ucl));
        assert!(out_of_control(7.4, lcl, ucl));
        assert!(!out_of_control(10.0, lcl, ucl));
    }

    #[test]
    fn no_samples_gives_zero_limits() {
        assert_eq!(control_limits(&[]), (0.0, 0.0, 0.0));
    }
}
