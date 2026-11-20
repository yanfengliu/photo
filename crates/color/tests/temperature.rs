//! Contracts of the temperature/tint model (DNG SDK `dng_temperature`).

use photo_color::temperature::{ISOTHERMS, temp_tint_to_xy, xy_to_temp_tint};
use photo_core::colorimetry::Chromaticity;

/// CIE 1960 uv from xy.
fn xy_to_uv(xy: Chromaticity) -> (f64, f64) {
    let d = 1.5 - xy.x + 6.0 * xy.y;
    (2.0 * xy.x / d, 3.0 * xy.y / d)
}

/// Krystek (1985): rational approximation of the Planckian locus in CIE 1960
/// uv, stated accurate to about 1e-4 between 1000 K and 15000 K.
fn krystek_uv(t: f64) -> (f64, f64) {
    let u = (0.860_117_757 + 1.541_182_54e-4 * t + 1.286_412_12e-7 * t * t)
        / (1.0 + 8.424_202_35e-4 * t + 7.081_451_63e-7 * t * t);
    let v = (0.317_398_726 + 4.228_062_45e-5 * t + 4.204_816_91e-8 * t * t)
        / (1.0 - 2.897_418_16e-5 * t + 1.614_560_53e-7 * t * t);
    (u, v)
}

/// Kim et al. (2002): cubic approximation of the Planckian locus in xy,
/// valid from 1667 K to 25000 K.
fn kim_xy(t: f64) -> Chromaticity {
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t.powi(3) - 0.234_358_9e6 / t.powi(2) + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t.powi(3) + 2.107_037_9e6 / t.powi(2) + 0.222_634_7e3 / t + 0.240_390
    };
    let y = if t <= 2222.0 {
        -1.106_381_4 * x.powi(3) - 1.348_110_20 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x.powi(3) - 1.374_185_93 * x.powi(2) + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x.powi(3) - 5.873_386_70 * x.powi(2) + 3.751_129_97 * x - 0.370_014_83
    };
    Chromaticity::new(x, y)
}

#[test]
fn every_isotherm_row_lies_on_the_planckian_locus() {
    // An independent formula per row, so a single mistyped digit cannot hide.
    for row in ISOTHERMS.iter() {
        if row.mired < 40.0 {
            continue; // hotter than either approximation is valid for
        }
        let kelvin = 1.0e6 / row.mired;
        let (u, v) = if kelvin <= 15_000.0 {
            krystek_uv(kelvin)
        } else {
            xy_to_uv(kim_xy(kelvin))
        };
        let error = ((row.u - u).powi(2) + (row.v - v).powi(2)).sqrt();
        assert!(
            error < 2.0e-4,
            "row at {} mired is {error:e} away from the locus",
            row.mired
        );
    }
}

#[test]
fn the_isotherm_table_is_smooth() {
    // Covers the rows the locus formulas cannot reach, and catches the
    // 325-mired entry that circulates as 0.24702 (0.24792 fits its
    // neighbours). Third differences are taken within each run of four
    // evenly spaced rows. The locus itself bends by up to 2e-4 over three
    // 25-mired steps, so the bound is 4e-4: a row that is off by more than
    // about 1e-4 fails.
    let mut checked = 0;
    for window in ISOTHERMS.windows(4) {
        let step = window[1].mired - window[0].mired;
        if window
            .windows(2)
            .any(|pair| (pair[1].mired - pair[0].mired - step).abs() > 1e-9)
        {
            continue;
        }
        let third = |f: fn(&photo_color::temperature::Isotherm) -> f64| {
            (f(&window[3]) - 3.0 * f(&window[2]) + 3.0 * f(&window[1]) - f(&window[0])).abs()
        };
        let at = window[1].mired;
        assert!(
            third(|row| row.u) < 4.0e-4,
            "u is not smooth near {at} mired"
        );
        assert!(
            third(|row| row.v) < 4.0e-4,
            "v is not smooth near {at} mired"
        );
        checked += 1;
    }
    assert!(checked >= 24, "only {checked} windows were checked");
}

#[test]
fn isotherm_slopes_are_perpendicular_to_the_locus() {
    for row in ISOTHERMS.iter() {
        let kelvin = 1.0e6 / row.mired.max(1.0);
        if !(1700.0..=14_000.0).contains(&kelvin) {
            continue;
        }
        let (u0, v0) = krystek_uv(kelvin * 0.999);
        let (u1, v1) = krystek_uv(kelvin * 1.001);
        // The isotherm direction (1, slope) must be normal to the tangent.
        let (tu, tv) = (u1 - u0, v1 - v0);
        let cosine = (tu + tv * row.slope)
            / ((tu * tu + tv * tv).sqrt() * (1.0 + row.slope * row.slope).sqrt());
        assert!(
            cosine.abs() < 0.02,
            "row at {} mired: cos = {cosine}",
            row.mired
        );
    }
}

#[test]
fn zero_tint_puts_the_chromaticity_on_the_locus() {
    for kelvin in [2000.0, 2850.0, 4000.0, 5500.0, 6500.0, 9000.0, 14_000.0] {
        let (u, v) = xy_to_uv(temp_tint_to_xy(kelvin, 0.0));
        let (ku, kv) = krystek_uv(kelvin);
        assert!((u - ku).abs() < 4e-4 && (v - kv).abs() < 4e-4, "{kelvin} K");
    }
}

#[test]
fn temperature_and_tint_round_trip_through_xy() {
    for kelvin in [
        2000.0, 2500.0, 3200.0, 4100.0, 5000.0, 5500.0, 6500.0, 7500.0, 10_000.0, 20_000.0,
        50_000.0,
    ] {
        for tint in [-150.0, -60.0, -10.0, 0.0, 10.0, 60.0, 150.0] {
            let (back_kelvin, back_tint) = xy_to_temp_tint(temp_tint_to_xy(kelvin, tint));
            assert!(
                (back_kelvin / kelvin - 1.0).abs() < 2e-3,
                "{kelvin} K, tint {tint}: temperature came back as {back_kelvin}"
            );
            assert!(
                (back_tint - tint).abs() < 0.2,
                "{kelvin} K, tint {tint}: tint came back as {back_tint}"
            );
        }
    }
}

#[test]
fn xy_round_trips_through_temperature_and_tint() {
    for xy in [
        Chromaticity::D65,
        Chromaticity::D50,
        Chromaticity::new(0.4476, 0.4074), // illuminant A
        Chromaticity::new(0.3101, 0.3162), // illuminant C
        Chromaticity::new(0.3800, 0.3600),
        Chromaticity::new(0.2900, 0.3200),
    ] {
        let (kelvin, tint) = xy_to_temp_tint(xy);
        let back = temp_tint_to_xy(kelvin, tint);
        assert!(
            (back.x - xy.x).abs() < 1e-5 && (back.y - xy.y).abs() < 1e-5,
            "{xy:?} -> {back:?}"
        );
    }
}

#[test]
fn the_standard_illuminants_get_their_familiar_temperatures() {
    let (d65, d65_tint) = xy_to_temp_tint(Chromaticity::D65);
    assert!((d65 - 6504.0).abs() < 40.0, "D65 read as {d65} K");
    assert!(d65_tint.abs() < 15.0);
    let (d50, _) = xy_to_temp_tint(Chromaticity::D50);
    assert!((d50 - 5003.0).abs() < 40.0, "D50 read as {d50} K");
    let (a, a_tint) = xy_to_temp_tint(Chromaticity::new(0.4476, 0.4074));
    assert!((a - 2856.0).abs() < 25.0, "illuminant A read as {a} K");
    assert!(a_tint.abs() < 3.0);
}

#[test]
fn warmer_is_redder_and_positive_tint_is_greener_light() {
    // Lower temperature means a redder illuminant (larger x).
    assert!(temp_tint_to_xy(3000.0, 0.0).x > temp_tint_to_xy(6500.0, 0.0).x);
    // Positive tint describes a greener illuminant (the correction for it is
    // magenta): in CIE 1960 uv green is toward larger v and smaller u.
    let (u0, v0) = xy_to_uv(temp_tint_to_xy(5500.0, 0.0));
    let (u1, v1) = xy_to_uv(temp_tint_to_xy(5500.0, 60.0));
    assert!(v1 > v0 && u1 < u0);
    let distance = ((u1 - u0).powi(2) + (v1 - v0).powi(2)).sqrt();
    assert!(
        (distance - 60.0 / 3000.0).abs() < 1e-6,
        "tint is 3000 x delta-uv"
    );
}

#[test]
fn out_of_range_requests_stay_finite() {
    for (kelvin, tint) in [
        (0.0, 0.0),
        (-50.0, 10.0),
        (1.0e9, -400.0),
        (f64::NAN, 0.0),
        (500.0, f64::NAN),
    ] {
        let xy = temp_tint_to_xy(kelvin, tint);
        assert!(
            xy.x.is_finite() && xy.y.is_finite(),
            "{kelvin} K, tint {tint}"
        );
    }
    let (kelvin, tint) = xy_to_temp_tint(Chromaticity::new(0.9, 0.05));
    assert!(kelvin.is_finite() && tint.is_finite());
}
