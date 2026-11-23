use pet_core::sprite::{BODY, CHEEKS};
use pet_core::*;
use std::time::{Duration, Instant};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn body_face(pose: Pose) -> Vec<String> {
    rasterize(&pose, &Palette::default())
        .to_ascii()
        .lines()
        .skip(10)
        .take(6)
        .map(|row| row[1..17].to_string())
        .collect()
}

#[test]
fn golden_faces_and_unchanged_body() {
    let cases = [
        (
            Pose::default(),
            [
                "o##############o",
                "o###EE####EE###o",
                "o###EE####EE###o",
                "o#CC########CC#o",
                "o#####M##M#####o",
                "o######MM######o",
            ],
        ),
        (
            Pose {
                eyes: EyeKind::Half,
                ..Pose::default()
            },
            [
                "o##############o",
                "o##############o",
                "o###EE####EE###o",
                "o#CC########CC#o",
                "o#####M##M#####o",
                "o######MM######o",
            ],
        ),
        (
            Pose {
                look: (1, -1),
                mouth: MouthKind::Flat,
                ..Pose::default()
            },
            [
                "o####EE####EE##o",
                "o####EE####EE##o",
                "o##############o",
                "o#CC########CC#o",
                "o######MM######o",
                "o##############o",
            ],
        ),
        (
            Pose {
                mouth: MouthKind::O,
                ..Pose::default()
            },
            [
                "o##############o",
                "o###EE####EE###o",
                "o###EE####EE###o",
                "o#CC########CC#o",
                "o######MM######o",
                "o######MM######o",
            ],
        ),
        (
            Pose {
                eyes: EyeKind::Happy,
                mouth: MouthKind::Open,
                ..Pose::default()
            },
            [
                "o##############o",
                "o###E######E###o",
                "o##E#E####E#E##o",
                "o#CC########CC#o",
                "o#####MMMM#####o",
                "o######MM######o",
            ],
        ),
        (
            Pose {
                eyes: EyeKind::X,
                mouth: MouthKind::Frown,
                cheeks: false,
                ..Pose::default()
            },
            [
                "o##############o",
                "o##E#E####E#E##o",
                "o###E######E###o",
                "o##E#E####E#E##o",
                "o######MM######o",
                "o#####M##M#####o",
            ],
        ),
        (
            Pose {
                eyes: EyeKind::Half,
                mouth: MouthKind::Flat,
                ..Pose::default()
            },
            [
                "o##############o",
                "o##############o",
                "o###EE####EE###o",
                "o#CC########CC#o",
                "o######MM######o",
                "o##############o",
            ],
        ),
    ];
    for (pose, face) in cases {
        assert_eq!(body_face(pose), face);
        let ascii = rasterize(&pose, &Palette::default()).to_ascii();
        for (y, row) in ascii.lines().skip(6).take(12).enumerate() {
            let restored: String = row[1..17]
                .chars()
                .map(|c| if "ECM".contains(c) { '#' } else { c })
                .collect();
            assert_eq!(restored, BODY[y]);
        }
    }
}

#[test]
fn legal_face_parts_are_inside_fill_disjoint_and_symmetric() {
    for eyes in [EyeKind::Open, EyeKind::Half, EyeKind::Happy, EyeKind::X] {
        for mouth in [
            MouthKind::Smile,
            MouthKind::Flat,
            MouthKind::Open,
            MouthKind::O,
            MouthKind::Frown,
        ] {
            for cheeks in [false, true] {
                if eyes == EyeKind::X && cheeks {
                    continue;
                }
                for dx in -1..=1 {
                    for dy in -1..=0 {
                        let mut cells = eyes.cells((dx, dy));
                        cells.extend(mouth.cells());
                        if cheeks {
                            cells.extend(CHEEKS);
                        }
                        let mut unique = std::collections::HashSet::new();
                        for &(x, y) in &cells {
                            assert_eq!(BODY[y as usize].as_bytes()[x as usize], b'#');
                            assert!(unique.insert((x, y)), "overlap {eyes:?} {mouth:?}");
                        }
                        if dx == 0 && dy == 0 {
                            for &(x, y) in &cells {
                                assert!(unique.contains(&(15 - x, y)));
                            }
                        }
                    }
                }
            }
        }
    }
    let x = Pose {
        eyes: EyeKind::X,
        ..Pose::default()
    };
    assert_eq!(
        rasterize(&x, &Palette::default()),
        rasterize(&Pose { cheeks: false, ..x }, &Palette::default())
    );
}

#[test]
fn effects_and_all_motion_stay_in_bounds_and_rects_never_overlap() {
    let mut effects = vec![
        Fx::None,
        Fx::Exclaim { error: false },
        Fx::Exclaim { error: true },
    ];
    for i in 0..=3 {
        effects.extend([Fx::Dots(i), Fx::Heart(i), Fx::Zzz(i)]);
    }
    for fx in effects {
        for shake in -1..=1 {
            for hop in 0..=2 {
                for bob in 0..=1 {
                    let frame = rasterize(
                        &Pose {
                            fx,
                            shake,
                            hop,
                            bob,
                            ..Pose::default()
                        },
                        &Palette::default(),
                    );
                    assert!(frame.rects.len() < 120, "{}", frame.rects.len());
                    let mut seen = std::collections::HashSet::new();
                    for r in &frame.rects {
                        for y in r.y..r.y + r.h {
                            for x in r.x..r.x + r.w {
                                assert!(
                                    (0..CANVAS_W as i16).contains(&x)
                                        && (0..CANVAS_H as i16).contains(&y)
                                );
                                assert!(seen.insert((x, y)));
                            }
                        }
                    }
                    for (y, row) in frame.to_ascii().lines().enumerate() {
                        for (x, c) in row.bytes().enumerate() {
                            if b"wdzH!X".contains(&c) {
                                assert!((18..=24).contains(&x) && y <= 11);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn priority_and_independent_deadlines_restore_live_business_state() {
    let now = Instant::now();
    let mut state = StateMachine::new(Config::default(), now);
    state.event(PetEvent::TaskStarted, now);
    state.feedback(true, now);
    assert_eq!(state.tick(now).0, PetState::Error);
    state.event(PetEvent::NeedsApproval, now);
    assert_eq!(state.tick(now).0, PetState::Attention);
    state.poke(now);
    assert_eq!(state.tick(now).0, PetState::Poked);
    state.event(PetEvent::DebugState(PetState::Sleep), now);
    assert_eq!(state.tick(now).0, PetState::Sleep);
    state.drag(true, now);
    assert_eq!(state.tick(now).0, PetState::Dragged);
    state.drag(false, now + ms(3100));
    assert_eq!(state.tick(now + ms(3100)).0, PetState::Attention);
    state.event(PetEvent::ApprovalResolved, now + ms(3100));
    assert_eq!(state.tick(now + ms(3100)).0, PetState::Error);
    assert_eq!(state.tick(now + ms(8000)).0, PetState::Working);
    state.feedback(true, now + ms(8100));
    state.event(PetEvent::TaskStarted, now + ms(8200));
    assert_eq!(state.tick(now + ms(8200)).0, PetState::Working);
}

#[test]
fn sleep_wakes_for_all_events_and_only_discrete_work_times_out() {
    let now = Instant::now();
    let cfg = Config {
        sleep_after: ms(1000),
        working_timeout: ms(2000),
    };
    let mut state = StateMachine::new(cfg, now);
    assert_eq!(state.tick(now).1, Some(now + ms(1000)));
    assert_eq!(state.tick(now + ms(1000)).0, PetState::Sleep);
    state.event(PetEvent::ApprovalResolved, now + ms(1100));
    assert_eq!(state.tick(now + ms(1100)).0, PetState::Idle);
    state.event(PetEvent::TaskStarted, now + ms(1200));
    assert_eq!(state.tick(now + ms(3200)).0, PetState::Sleep);
    state.sync_activity(true, true, now + ms(4000));
    assert_eq!(state.tick(now + ms(10000)).0, PetState::Attention);
    state.sync_activity(true, false, now + ms(11000));
    assert_eq!(state.tick(now + ms(20000)).0, PetState::Working);
    state.feedback(false, now + ms(20000));
    assert_eq!(state.tick(now + ms(22500)).0, PetState::Working);
    state.sync_activity(false, false, now + ms(23000));
    state.sync_activity(false, false, now + ms(23900));
    assert_eq!(state.tick(now + ms(24000)).0, PetState::Sleep);
}

#[test]
fn ten_minute_animations_have_strict_deadlines_and_no_intermediate_changes() {
    for state in [
        PetState::Idle,
        PetState::Working,
        PetState::Attention,
        PetState::Success,
        PetState::Error,
        PetState::Sleep,
        PetState::Poked,
        PetState::Dragged,
    ] {
        let start = Instant::now();
        let mut anim = Animator::new(0, start);
        anim.set_state(state, start);
        let mut now = start;
        let mut wakes = 0;
        while now < start + Duration::from_secs(600) {
            let tick = anim.tick(now);
            assert!(
                tick.pose.bob <= 1 && tick.pose.hop <= 2 && (-1..=1).contains(&tick.pose.shake)
            );
            assert!((-1..=1).contains(&tick.pose.look.0) && (-1..=0).contains(&tick.pose.look.1));
            assert!(rasterize(&tick.pose, &Palette::default()).rects.len() < 120);
            if let Some(next) = tick.next_wake {
                assert!(next > now);
                let before = next - Duration::from_nanos(1);
                assert_eq!(anim.tick(before).pose, tick.pose, "{state:?}");
                anim.set_state(state, before);
                assert_eq!(anim.tick(before).next_wake, Some(next));
                now = next;
                wakes += 1;
            } else {
                assert_eq!(anim.tick(start + Duration::from_secs(600)).pose, tick.pose);
                break;
            }
        }
        if state == PetState::Sleep {
            assert!(wakes < 430);
        }
        if state == PetState::Idle {
            assert!((1200..1800).contains(&wakes));
        }
    }
}

#[test]
fn blink_intervals_and_late_ticks_are_deterministic() {
    for state in [PetState::Idle, PetState::Working] {
        let start = Instant::now();
        let mut anim = Animator::new(42, start);
        anim.set_state(state, start);
        let mut now = start;
        let mut last_start = start;
        let mut blinking = false;
        while now < start + Duration::from_secs(600) {
            let out = anim.tick(now);
            let closed = out.pose.eyes == EyeKind::Half;
            if closed && !blinking {
                let interval = now.duration_since(last_start);
                let min = if state == PetState::Working {
                    3000
                } else {
                    2000
                };
                assert!((ms(min)..=ms(min + 3000)).contains(&interval));
                last_start = now;
            } else if blinking && !closed {
                assert_eq!(now - last_start, ms(120));
            }
            blinking = closed;
            now = out.next_wake.unwrap();
        }
        // A single late tick must match a complete schedule of on-time ticks.
        let mut direct = Animator::new(42, start);
        direct.set_state(state, start);
        assert_eq!(direct.tick(now).pose, anim.tick(now).pose);
        assert_eq!(direct.tick(now).next_wake, anim.tick(now).next_wake);
        assert!(direct.tick(now).next_wake.unwrap() > now);
    }
}
