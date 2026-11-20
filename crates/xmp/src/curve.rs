//! Point curves as Camera Raw writes them: an ordered list of `x, y` pairs
//! on a scale of 0 to 255.
//!
//! Points are written as whole levels, the only form other applications are
//! known to produce, so a point that lies between two levels is stored at
//! the nearer one. A curve whose points sit on levels round-trips exactly.

use photo_core::develop::{CurvePoint, PointCurve, ToneCurve};

use crate::rdf::{CRS, Container, Item, Packet};
use crate::values::parse_number;

/// The top of the scale sidecars use, standing for 1.0.
const SCALE: f32 = 255.0;

/// The curve properties, in the order of [`channels`].
const PROPERTIES: [&str; 4] = [
    "ToneCurvePV2012",
    "ToneCurvePV2012Red",
    "ToneCurvePV2012Green",
    "ToneCurvePV2012Blue",
];

fn channels(curve: &ToneCurve) -> [&PointCurve; 4] {
    [&curve.rgb, &curve.red, &curve.green, &curve.blue]
}

fn channels_mut(curve: &mut ToneCurve) -> [&mut PointCurve; 4] {
    [
        &mut curve.rgb,
        &mut curve.red,
        &mut curve.green,
        &mut curve.blue,
    ]
}

/// A curve from its list items. Items that are not a pair of numbers are
/// skipped, and [`PointCurve::new`] repairs whatever is left.
fn decode(items: &[String]) -> PointCurve {
    PointCurve::new(items.iter().filter_map(|item| {
        let (x, y) = item.split_once(',')?;
        Some(CurvePoint::new(
            parse_number::<f32>(x)? / SCALE,
            parse_number::<f32>(y)? / SCALE,
        ))
    }))
}

/// The list items for a curve: its points at whole levels, without the
/// points that share a level with the one before.
fn encode(curve: &PointCurve) -> Vec<String> {
    // A float-to-integer cast saturates, and takes anything not a number to zero.
    let level = |value: f32| (value * SCALE).round() as u8;
    let mut levels: Vec<(u8, u8)> = Vec::new();
    for point in curve.points() {
        let x = level(point.x);
        if levels.last().is_none_or(|&(previous, _)| x > previous) {
            levels.push((x, level(point.y)));
        }
    }
    if levels.len() < 2 {
        levels = vec![(0, 0), (u8::MAX, u8::MAX)];
    }
    levels
        .into_iter()
        .map(|(x, y)| format!("{x}, {y}"))
        .collect()
}

/// Reads the point curves that are present. Returns whether any was.
pub(crate) fn read(packet: &Packet, curve: &mut ToneCurve) -> bool {
    let mut found = false;
    for (name, channel) in PROPERTIES.into_iter().zip(channels_mut(curve)) {
        if let Some(items) = packet.list(CRS.property(name)) {
            *channel = decode(&items);
            found = true;
        }
    }
    found
}

/// Writes the four point curves, leaving alone those the packet already
/// describes the same way.
pub(crate) fn write(packet: &mut Packet, curve: &ToneCurve) {
    for (name, channel) in PROPERTIES.into_iter().zip(channels(curve)) {
        let property = CRS.property(name);
        let wanted = encode(channel);
        let current = packet.list(property).map(|items| encode(&decode(&items)));
        if current.as_ref() != Some(&wanted) {
            let items: Vec<Item> = wanted.into_iter().map(Item::plain).collect();
            packet.set_list(property, Container::Seq, &items);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(x: u8, y: u8) -> CurvePoint {
        CurvePoint::new(f32::from(x) / SCALE, f32::from(y) / SCALE)
    }

    fn items(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn every_pair_of_levels_round_trips_exactly() {
        for x in 0..=u8::MAX {
            let y = x.wrapping_mul(97).wrapping_add(13);
            let curve = PointCurve::new([level(0, 3), level(x.clamp(1, 254), y), level(255, 250)]);
            assert_eq!(decode(&encode(&curve)), curve, "level {x}");
        }
    }

    #[test]
    fn the_identity_is_the_two_end_points() {
        assert_eq!(encode(&PointCurve::default()), items(&["0, 0", "255, 255"]));
        assert_eq!(decode(&items(&["0, 0", "255, 255"])), PointCurve::default());
    }

    #[test]
    fn items_are_read_tolerantly() {
        let curve = decode(&items(&[
            " 0 , 0 ",
            "64,51",
            "not a point",
            "128",
            "+192, 204.0",
            "255, 255",
        ]));
        assert_eq!(
            curve,
            PointCurve::new([level(0, 0), level(64, 51), level(192, 204), level(255, 255)])
        );
        assert_eq!(decode(&items(&["12, 40"])), PointCurve::default());
        assert_eq!(decode(&[]), PointCurve::default());
    }

    #[test]
    fn points_that_fall_on_one_level_are_written_once() {
        let crowded = PointCurve::new([
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.4),
            CurvePoint::new(0.5 + PointCurve::MIN_GAP, 0.6),
            CurvePoint::new(1.0, 1.0),
        ]);
        assert_eq!(crowded.points().len(), 4);
        assert_eq!(encode(&crowded), items(&["0, 0", "128, 102", "255, 255"]));

        let collapsed = PointCurve::new([
            CurvePoint::new(0.5, 0.2),
            CurvePoint::new(0.5 + PointCurve::MIN_GAP, 0.9),
        ]);
        assert_eq!(encode(&collapsed), items(&["0, 0", "255, 255"]));
    }
}
