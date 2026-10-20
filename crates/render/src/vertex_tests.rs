// SPDX-License-Identifier: GPL-3.0-or-later

//! Checks the vertices the GPU gets back in clip space: one just behind the eye, whose screen
//! position is hundreds of millions of pixels out, stays where its clip coordinates put it, so
//! the GPU clips its triangles as the GameCube does.

use ssbm_gx::xform::{Output, Viewport};

use crate::{EFB_HEIGHT, EFB_WIDTH, Renderer, snap};

/// The game camera's viewport in a Final Destination frame (melee-hd's ganon-captain-fd, tick
/// 6279), and the scissor's offset.
const VP: Viewport = Viewport {
    wd: 320.0,
    ht: -240.0,
    z_range: 16_777_215.0,
    x_orig: 662.0,
    y_orig: 582.0,
    far_z: 16_777_215.0,
};
const OFF: (f32, f32) = (342.0, 342.0);

fn vertex(clip: [f32; 4]) -> [f32; 4] {
    let o = Output {
        clip,
        ..Output::default()
    };
    Renderer::gpu_vertex(&o, VP.screen(clip), OFF, &VP).pos
}

/// Where the clip coordinates put the vertex in the GPU's clip space, made without dividing by
/// w, and without the snap's sixteenth.
fn exact(clip: [f32; 4]) -> [f64; 2] {
    let [x, y, _, w] = clip.map(f64::from);
    let (hw, hh) = (f64::from(EFB_WIDTH) / 2.0, f64::from(EFB_HEIGHT) / 2.0);
    let (wd, ht) = (f64::from(VP.wd), f64::from(VP.ht));
    let (xo, yo) = (
        f64::from(VP.x_orig - OFF.0) - hw,
        f64::from(VP.y_orig - OFF.1),
    );
    [(x * wd + xo * w) / hw, w - (y * ht + yo * w) / hh]
}

/// A vertex of Final Destination's background just behind the eye (w -0.00038, at -5.5e8 and
/// -7.6e8 pixels): snapped in integers its screen position saturated at 2^31 sixteenths, which
/// moved it from (656.5, -1209.8) to (160, -194) in clip space, so the part of its triangle in
/// front of the camera covered the screen nearer than the whole scene, and everything drawn
/// after it that tests depth failed (the stage, the fighters, the timer).
#[test]
fn a_vertex_just_behind_the_eye_stays_in_place() {
    let clip = [656.50977, -1209.8384, -1.0000331, -0.00038146973];
    let pos = vertex(clip);
    let [x, y] = exact(clip);
    for (got, want) in [(pos[0], x), (pos[1], y)] {
        assert!(
            (f64::from(got) - want).abs() <= want.abs() * 1e-5,
            "{pos:?} against ({x}, {y})"
        );
    }
    assert_eq!(pos[3], clip[3]);
}

/// A vertex in the eye's plane has no screen position, only a direction.
#[test]
fn a_vertex_in_the_eye_s_plane_is_a_direction() {
    let clip = [656.5, -1209.8, -1.0, 0.0];
    let pos = vertex(clip);
    let [x, y] = exact(clip);
    assert!(pos.iter().all(|v| v.is_finite()), "{pos:?}");
    for (got, want) in [(pos[0], x), (pos[1], y)] {
        assert!(
            (f64::from(got) - want).abs() <= want.abs() * 1e-6,
            "{pos:?} against ({x}, {y})"
        );
    }
}

/// Snapping in floats rounds every position the integers could hold as they did, bit for bit,
/// so the frames of vertices on and near the screen are as they were.
#[test]
fn the_snap_is_the_integer_snap_where_that_holds() {
    let integer = |v: f32| {
        let x = 16.0 * v;
        let t = x as i32;
        let r = if x - t as f32 >= 0.5 { t + 1 } else { t };
        (r - 1) as f32 / 16.0
    };
    let mut v = 1.0e-3f32;
    while v < 1.3e8 {
        for s in [
            v,
            -v,
            v + 0.5 / 16.0,
            -v - 0.5 / 16.0,
            v.next_up(),
            -v.next_up(),
        ] {
            assert_eq!(snap(s).to_bits(), integer(s).to_bits(), "{s}");
        }
        v *= 1.01;
    }
}
