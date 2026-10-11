//! Nowebiew 布局桥：把 taffy（flexbox/grid 规范实现）暴露成极小的 C ABI，
//! 供运行时（C）调用。M1 仅用 flexbox 子集。

use taffy::prelude::*;

/// 与 runtime/nw_ui.h 中 NWLayoutStyle 严格一一对应（repr(C)）
#[repr(C)]
pub struct NWLayoutStyle {
    pub display: i32,        // 0=flex 1=none
    pub flex_direction: i32, // 0=row 1=column 2=row-reverse 3=column-reverse
    pub justify: i32,        // 0=start 1=center 2=end 3=space-between 4=space-around 5=space-evenly
    pub align: i32,          // 0=start 1=center 2=end 3=stretch 4=baseline
    pub gap: f32,            // px
    pub padding: [f32; 4],   // top right bottom left (px)
    pub margin: [f32; 4],    // top right bottom left (px)
    pub width_kind: i32,     // 0=auto 1=px 2=percent
    pub width: f32,
    pub height_kind: i32,    // 0=auto 1=px 2=percent
    pub height: f32,
    pub flex_wrap: i32,      // 0=nowrap 1=wrap
    pub col_gap: f32,        // -1=用 gap
    pub row_gap: f32,        // -1=用 gap
    pub pos_kind: i32,       // 0=相对(流内) 1=absolute 2=fixed(绘制不随滚动)
    pub inset: [f32; 4],     // top right bottom left
    pub inset_kind: [i32; 4],// 0=auto 1=px 2=percent
    pub flex_grow: f32,      // 0=不增长（默认）
    pub min_w_kind: i32,     // 0=auto(min-content) 1=px 2=percent
    pub min_w: f32,
    pub min_h_kind: i32,
    pub min_h: f32,
    pub max_w_kind: i32,     // max-width: 0=none 1=px 2=percent
    pub max_w: f32,
    pub aspect: f32,         // aspect-ratio (w/h); 0=none. intrinsic ratio for replaced elements (img)
    pub margin_auto: i32,    // 位掩码：0=top 1=right 2=bottom 3=left 为 auto（水平 auto=居中）
}

fn map_style(s: &NWLayoutStyle) -> Style {
    let display = if s.display == 1 { Display::None } else { Display::Flex };
    let flex_direction = match s.flex_direction {
        1 => FlexDirection::Column,
        2 => FlexDirection::RowReverse,    /* float:right 行盒 */
        3 => FlexDirection::ColumnReverse,
        _ => FlexDirection::Row,
    };
    let justify_content = Some(match s.justify {
        1 => JustifyContent::CENTER,
        2 => JustifyContent::END,
        3 => JustifyContent::SPACE_BETWEEN,
        4 => JustifyContent::SPACE_AROUND,
        5 => JustifyContent::SPACE_EVENLY,
        _ => JustifyContent::START,
    });
    let align_items = Some(match s.align {
        1 => AlignItems::CENTER,
        2 => AlignItems::END,
        3 => AlignItems::STRETCH,
        4 => AlignItems::BASELINE,
        _ => AlignItems::START,
    });
    let gap = Size {
        width: LengthPercentage::length(if s.col_gap >= 0.0 { s.col_gap } else { s.gap }),
        height: LengthPercentage::length(if s.row_gap >= 0.0 { s.row_gap } else { s.gap }),
    };
    let padding = Rect {
        top: LengthPercentage::length(s.padding[0]),
        right: LengthPercentage::length(s.padding[1]),
        bottom: LengthPercentage::length(s.padding[2]),
        left: LengthPercentage::length(s.padding[3]),
    };
    /* margin:auto（浏览器块盒水平居中语义）：auto 侧交给 taffy 吸收自由空间 */
    let mg = |i: usize| -> LengthPercentageAuto {
        if s.margin_auto & (1 << i) != 0 { LengthPercentageAuto::auto() }
        else { LengthPercentageAuto::length(s.margin[i]) }
    };
    let margin = Rect {
        top: mg(0),
        right: mg(1),
        bottom: mg(2),
        left: mg(3),
    };
    let size = Size {
        width: dimension(s.width_kind, s.width),
        height: dimension(s.height_kind, s.height),
    };
    let flex_wrap = if s.flex_wrap == 1 { FlexWrap::Wrap } else { FlexWrap::NoWrap };
    let position = match s.pos_kind {
        1 | 2 => Position::Absolute,
        _ => Position::Relative,
    };
    let ip = |kind: i32, v: f32| -> LengthPercentageAuto {
        match kind {
            1 => LengthPercentageAuto::length(v),
            2 => LengthPercentageAuto::percent(v / 100.0),
            _ => LengthPercentageAuto::auto(),
        }
    };
    let inset = Rect {
        top: ip(s.inset_kind[0], s.inset[0]),
        right: ip(s.inset_kind[1], s.inset[1]),
        bottom: ip(s.inset_kind[2], s.inset[2]),
        left: ip(s.inset_kind[3], s.inset[3]),
    };
    let min_size = Size {
        width: ip(s.min_w_kind, s.min_w),
        height: ip(s.min_h_kind, s.min_h),
    };
    let max_size = Size {
        width: ip(s.max_w_kind, s.max_w),
        height: LengthPercentageAuto::auto(),
    };
    Style {
        display,
        flex_direction,
        justify_content,
        align_items,
        gap,
        padding,
        margin,
        size,
        flex_wrap,
        position,
        inset,
        flex_grow: s.flex_grow,
        min_size,
        max_size,
        aspect_ratio: if s.aspect > 0.0 { Some(s.aspect) } else { None },
        ..Default::default()
    }
}

fn dimension(kind: i32, v: f32) -> Dimension {
    match kind {
        1 => Dimension::length(v),
        2 => Dimension::percent(v / 100.0),
        _ => Dimension::auto(),
    }
}

fn tree_from(ptr: *mut TaffyTree) -> &'static mut TaffyTree {
    unsafe { &mut *ptr }
}

fn node(id: u32) -> NodeId {
    NodeId::new(id as u64)
}

#[no_mangle]
pub extern "C" fn nw_layout_new() -> *mut TaffyTree {
    Box::into_raw(Box::new(TaffyTree::new()))
}

/// 媒体段切换重建时释放整棵布局树（节点由调用方另行释放）
#[no_mangle]
pub extern "C" fn nw_layout_free(tree: *mut TaffyTree) {
    if tree.is_null() { return; }
    unsafe { drop(Box::from_raw(tree)); }
}

#[no_mangle]
pub extern "C" fn nw_layout_node(tree: *mut TaffyTree, s: *const NWLayoutStyle) -> u32 {
    let t = tree_from(tree);
    let style = map_style(unsafe { &*s });
    u64::from(t.new_leaf(style).expect("taffy new_leaf")) as u32
}

#[no_mangle]
pub extern "C" fn nw_layout_add_child(tree: *mut TaffyTree, parent: u32, child: u32) {
    let t = tree_from(tree);
    t.add_child(node(parent), node(child)).expect("taffy add_child");
}

/// 动态更新节点样式（textContent 变更、display 切换等）
#[no_mangle]
pub extern "C" fn nw_layout_set_style(tree: *mut TaffyTree, id: u32, s: *const NWLayoutStyle) {
    let t = tree_from(tree);
    let style = map_style(unsafe { &*s });
    t.set_style(node(id), style).expect("taffy set_style");
}

/// avail_w/avail_h: 可用空间；fit 为 1 时高度按内容收缩（fit-content，用于窗口自适应）
#[no_mangle]
pub extern "C" fn nw_layout_run(tree: *mut TaffyTree, root: u32, avail_w: f32, avail_h: f32, fit: i32) {
    let t = tree_from(tree);
    let w = AvailableSpace::Definite(avail_w.max(0.0));
    let h = if fit != 0 { AvailableSpace::MinContent } else { AvailableSpace::Definite(avail_h.max(0.0)) };
    t.compute_layout(node(root), Size { width: w, height: h }).expect("taffy compute_layout");
}

#[no_mangle]
pub extern "C" fn nw_layout_get(tree: *mut TaffyTree, node_id: u32, out: *mut f32) {
    let t = tree_from(tree);
    let l = t.layout(node(node_id)).expect("taffy layout");
    unsafe {
        // out: [x, y, w, h, content_w, content_h]
        *out.add(0) = l.location.x;
        *out.add(1) = l.location.y;
        *out.add(2) = l.size.width;
        *out.add(3) = l.size.height;
        *out.add(4) = l.size.width;
        *out.add(5) = l.size.height;
    }
}
