//! The aspect menu, on a 1200 x 800 frame.

use egui::{Vec2, vec2};
use photo_core::develop::CropConstraint;

use super::*;

const FRAME: Vec2 = vec2(1200.0, 800.0);

#[test]
fn the_menu_reads_a_stored_ratio_either_way_round() {
    assert_eq!(
        Aspect::of(CropConstraint::Original, FRAME),
        Some(Aspect::Original)
    );
    assert_eq!(Aspect::of(CropConstraint::Free, FRAME), None);
    let ratio = |width, height| CropConstraint::Ratio { width, height };
    assert_eq!(
        Aspect::of(ratio(16.0, 9.0), FRAME),
        Some(Aspect::Ratio(9.0, 16.0))
    );
    assert_eq!(
        Aspect::of(ratio(4.0, 5.0), FRAME),
        Some(Aspect::Ratio(4.0, 5.0))
    );
    assert_eq!(
        Aspect::of(ratio(3.0, 2.0), FRAME),
        Some(Aspect::Ratio(2.0, 3.0))
    );
    assert_eq!(
        Aspect::of(ratio(1200.0, 700.0), vec2(1200.0, 700.0)),
        Some(Aspect::AsShot)
    );
    assert_eq!(Aspect::of(ratio(2.2, 1.0), FRAME), Some(Aspect::Custom));

    let wide = Aspect::Ratio(9.0, 16.0).constraint(FRAME, true, (1.0, 1.0));
    assert_eq!(wide, ratio(16.0, 9.0));
    let tall = Aspect::Ratio(9.0, 16.0).constraint(FRAME, false, (1.0, 1.0));
    assert_eq!(tall, ratio(9.0, 16.0));
    assert_eq!(
        Aspect::Custom.constraint(FRAME, true, (7.0, 3.0)),
        ratio(7.0, 3.0)
    );
    assert_eq!(locked_ratio(CropConstraint::Original, FRAME), Some(1.5));
    assert_eq!(locked_ratio(CropConstraint::Free, FRAME), None);
    assert_eq!(Aspect::Ratio(2.0, 3.0).label(), "2 : 3");
}
