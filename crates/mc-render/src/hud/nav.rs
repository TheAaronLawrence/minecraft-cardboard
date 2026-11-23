//! Keyboard focus navigation, as vanilla's screens do it (`Screen.keyPressed`): Tab and
//! Shift+Tab step through the widgets in order (`ContainerEventHandler.handleTabNavigation`),
//! wrapping round; the arrows move to the nearest widget that way
//! (`handleArrowNavigation`), and inside a scrolling list from row to row, keeping the
//! column (`ContainerObjectSelectionList.nextFocusPath`).

/// `ScreenDirection`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

use Direction::*;

impl Direction {
    fn horizontal(self) -> bool {
        matches!(self, Left | Right)
    }

    fn positive(self) -> bool {
        matches!(self, Down | Right)
    }

    fn opposite(self) -> Direction {
        match self {
            Up => Down,
            Down => Up,
            Left => Right,
            Right => Left,
        }
    }

    /// `isAfter`: `a` is further this way than `b`.
    fn after(self, a: i32, b: i32) -> bool {
        if self.positive() { a > b } else { b > a }
    }

    /// The positive direction across this one: Right for Up and Down, Down for the others.
    fn across(self) -> Direction {
        if self.horizontal() { Down } else { Right }
    }
}

/// A widget that may take the focus: where it is (x, y, width, height), whether it's
/// active, and which row of the screen's scrolling list it's in, if it's in one.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub rect: [i32; 4],
    pub active: bool,
    pub row: Option<usize>,
}

/// `ScreenRectangle.getBoundInDirection`: the rectangle's last pixel that way.
fn bound([x, y, w, h]: [i32; 4], d: Direction) -> i32 {
    match d {
        Up => y,
        Down => y + h - 1,
        Left => x,
        Right => x + w - 1,
    }
}

/// `ScreenRectangle.getBorder`: the line of pixels just outside the rectangle that way.
fn border([x, y, w, h]: [i32; 4], d: Direction) -> [i32; 4] {
    match d {
        Up => [x, y - 1, w, 1],
        Down => [x, y + h, w, 1],
        Left => [x - 1, y, 1, h],
        Right => [x + w, y, 1, h],
    }
}

/// `ScreenRectangle.overlapsInAxis`, along the axis of `d`.
fn overlaps(a: [i32; 4], b: [i32; 4], d: Direction) -> bool {
    let (lo, hi) = if d.horizontal() {
        (Left, Right)
    } else {
        (Up, Down)
    };
    bound(a, lo).max(bound(b, lo)) <= bound(a, hi).min(bound(b, hi))
}

/// `ScreenRectangle.getCenterInAxis`, along the axis of `d`.
fn centre(r: [i32; 4], d: Direction) -> i32 {
    let (lo, hi) = if d.horizontal() {
        (Left, Right)
    } else {
        (Up, Down)
    };
    (bound(r, hi) + bound(r, lo)) / 2
}

/// Tab (or Shift+Tab, `forward` false) from the widget `focus`: the next active widget in
/// order, starting over from the first (or last) after the end (`Screen.keyPressed`).
pub fn tab(items: &[Item], focus: Option<usize>, forward: bool) -> Option<usize> {
    let after = |from: Option<usize>| -> Option<usize> {
        let order: Box<dyn Iterator<Item = usize>> = match (from, forward) {
            (Some(f), true) => Box::new(f + 1..items.len()),
            (Some(f), false) => Box::new((0..f).rev()),
            (None, true) => Box::new(0..items.len()),
            (None, false) => Box::new((0..items.len()).rev()),
        };
        order.into_iter().find(|&i| items[i].active)
    };
    after(focus).or_else(|| after(None))
}

/// A child of the screen: a widget outside the list, or the list.
#[derive(Clone, Copy, PartialEq)]
enum Child {
    Widget(usize),
    List,
}

/// An arrow key from the widget `focus` in a screen `screen` big whose scrolling list, if
/// any, is `list` (x, y, width, height): the widget it moves the focus to.
pub fn arrow(
    items: &[Item],
    focus: Option<usize>,
    d: Direction,
    screen: [i32; 4],
    list: [i32; 4],
) -> Option<usize> {
    // The list's rows, each its widgets in order.
    let mut rows: Vec<(usize, Vec<usize>)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(row) = item.row {
            match rows.last_mut() {
                Some((r, widgets)) if *r == row => widgets.push(i),
                _ => rows.push((row, vec![i])),
            }
        }
    }
    let mut children = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match item.row {
            None => children.push(Child::Widget(i)),
            Some(_) if !children.contains(&Child::List) => children.push(Child::List),
            Some(_) => {}
        }
    }
    let rect = |c: Child| match c {
        Child::Widget(i) => items[i].rect,
        Child::List => list,
    };
    // `ContainerObjectSelectionList.nextFocusPath`: down or up the rows from `from` (row,
    // column), into the widget in the same column or the last one, skipping rows where
    // that one is inactive. Into the list from outside: the first row's first widget, or
    // for Left its last, or for Up the last row's first.
    let in_list = |d: Direction, from: Option<(usize, usize)>| -> Option<usize> {
        let (column, d) = match (from, d) {
            (Some((_, c)), _) => (c, d),
            (None, Left) => (usize::MAX, Down),
            (None, Right) => (0, Down),
            (None, d) => (0, d),
        };
        let step: isize = match d {
            Up => -1,
            Down => 1,
            _ => return None,
        };
        let mut row = match from {
            Some((r, _)) => r as isize + step,
            None if step > 0 => 0,
            None => rows.len() as isize - 1,
        };
        while row >= 0 && (row as usize) < rows.len() {
            let widgets = &rows[row as usize].1;
            let i = widgets[column.min(widgets.len() - 1)];
            if items[i].active {
                return Some(i);
            }
            row += step;
        }
        None
    };
    let enter = |c: Child| match c {
        Child::Widget(i) => items[i].active.then_some(i),
        Child::List => in_list(d, None),
    };
    // `ContainerEventHandler.nextFocusPathInDirection`: the children across from the
    // focused one's edge and beyond it, nearest first then leftmost or topmost; failing
    // those, `nextFocusPathVaguelyInDirection`'s nearest beyond it, from side centre to
    // side centre.
    let geometric = |from: [i32; 4], excluded: Option<Child>| -> Option<usize> {
        let back = d.opposite();
        let first = bound(from, back);
        let mut near: Vec<Child> = children
            .iter()
            .copied()
            .filter(|&c| Some(c) != excluded)
            .filter(|&c| {
                let r = rect(c);
                let across = Direction::across(d);
                overlaps(r, from, across)
                    && (d.after(bound(r, back), first)
                        || bound(r, back) == first && d.after(bound(r, d), bound(from, d)))
            })
            .collect();
        let across = d.across();
        near.sort_by(|&a, &b| {
            let key = |c: Child| bound(rect(c), back);
            let (ka, kb) = (key(a), key(b));
            let primary = if d.positive() {
                ka.cmp(&kb)
            } else {
                kb.cmp(&ka)
            };
            primary.then(bound(rect(a), across.opposite()).cmp(&bound(rect(b), across.opposite())))
        });
        if let Some(i) = near.into_iter().find_map(enter) {
            return Some(i);
        }
        let point = |r: [i32; 4], side: Direction| -> (i64, i64) {
            let (along, other) = (bound(r, side) as i64, centre(r, d.across()) as i64);
            if d.horizontal() {
                (along, other)
            } else {
                (other, along)
            }
        };
        let (fx, fy) = point(from, d);
        let along = |(x, y): (i64, i64)| if d.horizontal() { x } else { y };
        let mut vague: Vec<(Child, i64)> = children
            .iter()
            .copied()
            .filter(|&c| Some(c) != excluded)
            .filter_map(|c| {
                let p = point(rect(c), back);
                d.after(along(p) as i32, along((fx, fy)) as i32)
                    .then(|| (c, (p.0 - fx).pow(2) + (p.1 - fy).pow(2)))
            })
            .collect();
        vague.sort_by_key(|&(_, distance)| distance);
        vague.into_iter().find_map(|(c, _)| enter(c))
    };
    match focus {
        // `Screen.handleArrowNavigation` with nothing focused: from just outside the
        // screen's opposite edge.
        None => geometric(border(screen, d.opposite()), None),
        Some(f) => match items[f].row {
            None => geometric(border(items[f].rect, d), Some(Child::Widget(f))),
            Some(row) => {
                let r = rows.iter().position(|(n, _)| *n == row)?;
                let widgets = &rows[r].1;
                let column = widgets.iter().position(|&i| i == f)?;
                // `ContainerObjectSelectionList.Entry.nextFocusPath`: along the row.
                let found = if d.horizontal() {
                    let step: isize = if d == Right { 1 } else { -1 };
                    let start = (column as isize + step).clamp(0, widgets.len() as isize - 1);
                    let mut i = start;
                    let mut found = None;
                    while i >= 0 && (i as usize) < widgets.len() {
                        let w = widgets[i as usize];
                        if i as usize != column && items[w].active {
                            found = Some(w);
                            break;
                        }
                        i += step;
                    }
                    found
                } else {
                    in_list(d, Some((r, column)))
                };
                found.or_else(|| geometric(border(list, d), Some(Child::List)))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(x: i32, y: i32, w: i32, row: Option<usize>) -> Item {
        Item {
            rect: [x, y, w, 20],
            active: true,
            row,
        }
    }

    const SCREEN: [i32; 4] = [0, 0, 480, 270];
    const LIST: [i32; 4] = [0, 33, 480, 204];

    /// Two columns of buttons in a list, with Done below it: down keeps the column and
    /// skips a row whose widget in it is inactive; left and right stay in the row; down
    /// off the last row reaches Done, and up from Done enters the last row.
    #[test]
    fn arrows_move_like_vanilla_in_a_list() {
        let mut items = vec![
            item(85, 37, 150, Some(0)),
            item(245, 37, 150, Some(0)),
            item(85, 62, 150, Some(1)),
            item(245, 62, 150, Some(1)),
            item(85, 87, 150, Some(2)),
            item(245, 87, 150, Some(2)),
            item(140, 244, 200, None),
        ];
        items[3].active = false;
        let go = |focus, d| arrow(&items, focus, d, SCREEN, LIST);
        assert_eq!(
            go(Some(1), Down),
            Some(5),
            "skips the inactive widget's row"
        );
        assert_eq!(go(Some(0), Down), Some(2));
        assert_eq!(go(Some(0), Right), Some(1));
        assert_eq!(go(Some(1), Right), None, "nothing right of the row's end");
        assert_eq!(go(Some(2), Right), None, "its neighbour is inactive");
        assert_eq!(go(Some(5), Down), Some(6), "off the list's bottom to Done");
        assert_eq!(go(Some(6), Up), Some(4), "into the last row's first widget");
        assert_eq!(go(None, Down), Some(0));
        assert_eq!(
            go(None, Up),
            Some(6),
            "from below the screen, the lowest first"
        );
    }

    /// Outside a list, the arrows go to the nearest widget that way whose span overlaps,
    /// or failing that the nearest from side centre to side centre.
    #[test]
    fn arrows_find_the_nearest_widget_on_a_grid() {
        // The title screen: two wide buttons over two half-width ones.
        let items = [
            item(140, 115, 200, None),
            item(140, 139, 200, None),
            item(140, 175, 98, None),
            item(242, 175, 98, None),
        ];
        let go = |focus, d| arrow(&items, focus, d, SCREEN, LIST);
        assert_eq!(
            go(Some(1), Down),
            Some(2),
            "the left of two below goes first"
        );
        assert_eq!(go(Some(3), Up), Some(1));
        assert_eq!(go(Some(2), Right), Some(3));
        assert_eq!(go(Some(3), Right), None);
        assert_eq!(go(Some(0), Left), None);
    }

    #[test]
    fn tab_skips_inactive_widgets_and_wraps() {
        let mut items = [
            item(0, 0, 10, None),
            item(0, 30, 10, None),
            item(0, 60, 10, None),
        ];
        items[1].active = false;
        assert_eq!(tab(&items, None, true), Some(0));
        assert_eq!(tab(&items, Some(0), true), Some(2));
        assert_eq!(tab(&items, Some(2), true), Some(0));
        assert_eq!(tab(&items, Some(0), false), Some(2));
        assert_eq!(tab(&items, None, false), Some(2));
    }
}
