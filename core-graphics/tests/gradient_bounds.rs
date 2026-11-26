#![cfg(target_os = "macos")]

extern crate core_foundation;
extern crate core_graphics;

use core_foundation::array::CFArray;
use core_graphics::base::{kCGImageAlphaPremultipliedLast, CGFloat};
use core_graphics::color::CGColor;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::CGPoint;
use core_graphics::gradient::{CGGradient, CGGradientDrawingOptions};

/// Red, then blue: two stops of a color space with three components and alpha.
const RED_THEN_BLUE: [CGFloat; 8] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0];
const ENDS: [CGFloat; 2] = [0.0, 1.0];

/// The pixels (RGBA) of `gradient` drawn from the left edge to the right edge of a row of
/// `WIDTH` pixels.
const WIDTH: usize = 16;

fn row_of(gradient: &CGGradient) -> Vec<[u8; 4]> {
    let context = CGContext::create_bitmap_context(
        None,
        WIDTH,
        1,
        8,
        0,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedLast,
    );
    context.draw_linear_gradient(
        gradient,
        CGPoint::new(0.0, 0.0),
        CGPoint::new(WIDTH as CGFloat, 0.0),
        CGGradientDrawingOptions::empty(),
    );
    let data = context.create_image().expect("an image of the row").data();
    data.bytes()[..WIDTH * 4]
        .chunks(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
        .collect()
}

fn assert_red_to_blue(row: &[[u8; 4]]) {
    let (first, middle, last) = (row[0], row[WIDTH / 2], row[WIDTH - 1]);
    assert!(first[0] > 200 && first[2] < 50, "starts red: {:?}", row);
    assert!(last[0] < 50 && last[2] > 200, "ends blue: {:?}", row);
    assert!(
        (90..170).contains(&middle[0]) && (90..170).contains(&middle[2]),
        "half way in the middle: {:?}",
        row
    );
    assert!(row.iter().all(|pixel| pixel[3] == 255), "opaque: {:?}", row);
}

fn red_and_blue() -> CFArray<CGColor> {
    CFArray::from_CFTypes(&[CGColor::rgb(1.0, 0.0, 0.0, 1.0), CGColor::rgb(0.0, 0.0, 1.0, 1.0)])
}

#[test]
fn a_gradient_of_components_is_drawn_from_its_first_stop_to_its_last() {
    let rgb = CGColorSpace::create_device_rgb();
    let gradient = CGGradient::create_with_color_components(&rgb, &RED_THEN_BLUE, &ENDS, 2);
    assert_red_to_blue(&row_of(&gradient));
}

#[test]
fn a_gradient_of_colors_is_drawn_from_its_first_stop_to_its_last() {
    let rgb = CGColorSpace::create_device_rgb();
    let gradient = CGGradient::create_with_colors(&rgb, &red_and_blue(), &ENDS);
    assert_red_to_blue(&row_of(&gradient));
}

#[test]
fn stops_without_locations_are_spread_evenly() {
    let rgb = CGColorSpace::create_device_rgb();
    let placed = row_of(&CGGradient::create_with_color_components(&rgb, &RED_THEN_BLUE, &ENDS, 2));
    assert_eq!(
        row_of(&CGGradient::create_with_color_components(&rgb, &RED_THEN_BLUE, &[], 2)),
        placed
    );
    // The colors are of another color space than the components above, so they are compared
    // with themselves.
    assert_eq!(
        row_of(&CGGradient::create_with_colors(&rgb, &red_and_blue(), &[])),
        row_of(&CGGradient::create_with_colors(&rgb, &red_and_blue(), &ENDS))
    );
}

#[test]
fn the_components_of_a_stop_follow_the_color_space() {
    assert_eq!(CGColorSpace::create_device_rgb().number_of_components(), 3);
    let gray = CGColorSpace::create_device_gray();
    assert_eq!(gray.number_of_components(), 1);
    // White, then black: gray and alpha for each stop.
    let gradient = CGGradient::create_with_color_components(&gray, &[1.0, 1.0, 0.0, 1.0], &ENDS, 2);
    let row = row_of(&gradient);
    assert!(row[0][0] > 200 && row[WIDTH - 1][0] < 50, "{:?}", row);
}

// In the tests below the slice handed over is the start of a longer array, so that a constructor
// that reads past the slice still reads memory of the test.

#[test]
#[should_panic(expected = "2 stops need 8 components, got 4")]
fn components_for_fewer_stops_than_counted_are_refused() {
    let rgb = CGColorSpace::create_device_rgb();
    CGGradient::create_with_color_components(&rgb, &RED_THEN_BLUE[..4], &ENDS, 2);
}

#[test]
#[should_panic(expected = "2 stops need 4 components, got 3")]
fn components_are_counted_by_the_color_space() {
    let gray = CGColorSpace::create_device_gray();
    CGGradient::create_with_color_components(&gray, &RED_THEN_BLUE[..3], &ENDS, 2);
}

#[test]
#[should_panic(expected = "2 stops need 2 locations, got 1")]
fn locations_for_fewer_stops_than_counted_are_refused() {
    let rgb = CGColorSpace::create_device_rgb();
    CGGradient::create_with_color_components(&rgb, &RED_THEN_BLUE, &ENDS[..1], 2);
}

#[test]
#[should_panic(expected = "2 stops need 2 locations, got 1")]
fn locations_for_fewer_stops_than_colors_are_refused() {
    let rgb = CGColorSpace::create_device_rgb();
    CGGradient::create_with_colors(&rgb, &red_and_blue(), &ENDS[..1]);
}

#[test]
#[should_panic(expected = "cannot be counted")]
fn a_count_whose_components_overflow_is_refused() {
    let rgb = CGColorSpace::create_device_rgb();
    CGGradient::create_with_color_components(&rgb, &RED_THEN_BLUE, &ENDS, usize::MAX);
}

#[test]
#[should_panic(expected = "!result.is_null()")]
fn no_stops_make_no_gradient() {
    let rgb = CGColorSpace::create_device_rgb();
    CGGradient::create_with_color_components(&rgb, &[], &[], 0);
}
