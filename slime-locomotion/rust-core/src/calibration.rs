//! Pure calculations for guided sensor calibration. Inputs must be fresh samples
//! selected by the caller; this module never reads devices or applies settings.
use serde_json::{json, Value};

pub fn jump_settings(source: &str, baseline: f64, peak: f64, samples: usize) -> Result<Value, String> {
    if samples < 4 || !baseline.is_finite() || !peak.is_finite() {
        return Err("Collect at least four fresh, valid samples.".into());
    }
    let delta = peak - baseline;
    let (threshold, release) = match source {
        "height" => {
            if !(0.05..=4.).contains(&baseline) || !(0.05..=4.).contains(&peak) || !(0.08..=2.).contains(&delta) {
                return Err("No clear headset height rise detected. Stand still, then jump again.".into());
            }
            let threshold = (delta * 0.45).clamp(0.04, 0.5);
            (threshold, threshold * 0.4)
        }
        "acceleration" => {
            if !(0. ..=30.).contains(&baseline) || !(0. ..=100.).contains(&peak) || delta < 1.5f64.max(baseline * 2.) {
                return Err("No clear chest acceleration peak detected. Stand still, then jump again.".into());
            }
            (baseline + 0.55 * delta, baseline + 0.2 * delta)
        }
        _ => return Err("Choose headset height or chest acceleration.".into()),
    };
    Ok(json!({"enabled":true,"source":source,"accel_axis":"magnitude","threshold":threshold,"release":release,"pulse_seconds":0.25,"cooldown_seconds":0.8,"height_window_seconds":0.15,"accel_window_seconds":0.06}))
}

pub fn direction_matches(name: &str, xy: [f64; 2]) -> bool {
    if xy.iter().any(|x| !x.is_finite() || x.abs() > 1.) { return false; }
    match name {
        "forward" => xy[1] >= 0.03 && xy[0].abs() <= 0.08,
        "backward" | "back" => xy[1] <= -0.03 && xy[0].abs() <= 0.08,
        "right" => xy[0] >= 0.03 && xy[1].abs() <= 0.08,
        "left" => xy[0] <= -0.03 && xy[1].abs() <= 0.08,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn jump_height_uses_rise_and_rejects_noise() {
        let s=jump_settings("height",1.6,1.8,8).unwrap();
        assert!((s["threshold"].as_f64().unwrap()-0.09).abs()<1e-9);
        assert!(jump_settings("height",1.6,1.65,8).is_err());
        assert!(jump_settings("height",1.6,1.8,3).is_err());
        assert!(jump_settings("height",f64::NAN,1.8,8).is_err());
        assert!(jump_settings("height",1.6,4.1,8).is_err());
    }
    #[test] fn acceleration_fallback_uses_real_peak_not_height() {
        let s=jump_settings("acceleration",0.4,4.4,5).unwrap();
        assert_eq!(s["source"],"acceleration");assert!((s["threshold"].as_f64().unwrap()-2.6).abs()<1e-9);
        assert!((s["release"].as_f64().unwrap()-1.2).abs()<1e-9);
        assert!(jump_settings("acceleration",1.,2.,8).is_err());
        assert!(jump_settings("acceleration",2.,5.,8).is_err());
        assert!(jump_settings("acceleration",0.,f64::INFINITY,8).is_err());
        assert!(jump_settings("height",1.6,1.8,0).is_err());
    }
    #[test] fn all_directions_require_expected_sign_and_small_cross_axis() {
        for (name,v) in [("forward",[0.,0.2]),("backward",[0.,-0.2]),("right",[0.2,0.]),("left",[-0.2,0.])] {assert!(direction_matches(name,v));assert!(!direction_matches(name,[-v[0],-v[1]]));}
        assert!(!direction_matches("forward",[0.2,0.2]));assert!(!direction_matches("forward",[0.,0.01]));assert!(!direction_matches("right",[f64::NAN,0.]));assert!(!direction_matches("unknown",[0.,0.2]));
    }
}
