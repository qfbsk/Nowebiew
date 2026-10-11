/* 实测 taffy 0.14 absolute 定位语义：
 * 1. 单边 inset（bottom+left，size auto）→ 期望 shrink-to-fit
 * 2. 显式 width + right → 期望 x = 容器宽 - width - right
 * 3. width:0 → 期望 0
 */
use taffy::prelude::*;

fn main() {
    let mut t: TaffyTree = TaffyTree::new();

    // 容器 900x640，列方向
    let container = t
        .new_leaf(Style {
            size: Size { width: length(900.0), height: length(640.0) },
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::STRETCH),
            ..Default::default()
        })
        .unwrap();

    // nav：absolute，bottom:16 left:20，size auto，内部横向 flex
    let c1 = t.new_leaf(Style {
        position: Position::Absolute,
        inset: Rect {
            top: auto(),
            right: auto(),
            bottom: length(16.0),
            left: length(20.0),
        },
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        gap: length(7.0),
        ..Default::default()
    }).unwrap();
    let b1 = t.new_leaf(Style { size: Size { width: length(34.0), height: length(34.0) }, ..Default::default() }).unwrap();
    let b2 = t.new_leaf(Style { size: Size { width: length(34.0), height: length(34.0) }, ..Default::default() }).unwrap();
    t.add_child(c1, b1).unwrap();
    t.add_child(c1, b2).unwrap();
    t.add_child(container, c1).unwrap();

    // themeBtn：absolute，top:14 right:18，width auto height 32
    let c2 = t.new_leaf(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(14.0),
            right: length(18.0),
            bottom: auto(),
            left: auto(),
        },
        size: Size { width: auto(), height: length(32.0) },
        ..Default::default()
    }).unwrap();
    t.add_child(container, c2).unwrap();

    // progress：absolute，width 0，height 3
    let c3 = t.new_leaf(Style {
        position: Position::Absolute,
        inset: Rect { top: length(0.0), left: length(0.0), right: auto(), bottom: auto() },
        size: Size { width: length(0.0), height: length(3.0) },
        ..Default::default()
    }).unwrap();
    t.add_child(container, c3).unwrap();

    t.compute_layout(container, Size {
        width: AvailableSpace::Definite(900.0),
        height: AvailableSpace::Definite(640.0),
    }).unwrap();

    for (name, id) in [("nav", c1), ("themeBtn", c2), ("progress", c3)] {
        let l = t.layout(id).unwrap();
        println!(
            "{}: x={:.1} y={:.1} w={:.1} h={:.1}",
            name, l.location.x, l.location.y, l.size.width, l.size.height
        );
    }
}
