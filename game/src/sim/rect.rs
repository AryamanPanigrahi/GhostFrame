//! Axis-aligned bounding boxes (AABB): rectangles that never rotate, so two
//! of them overlap exactly when they overlap on the x axis *and* the y axis.

/// Edges in subpixels. y grows downward, so `top < bottom`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// Strict overlap: boxes that only touch along an edge don't count, so a
    /// hitbox ending exactly where a hurtbox starts is a miss.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn overlapping_boxes() {
        assert!(rect(0, 0, 10, 10).overlaps(&rect(5, 5, 15, 15)));
        // Containment counts too.
        assert!(rect(0, 0, 10, 10).overlaps(&rect(2, 2, 3, 3)));
    }

    #[test]
    fn separated_on_one_axis_is_a_miss() {
        // Same height, but side by side.
        assert!(!rect(0, 0, 10, 10).overlaps(&rect(20, 0, 30, 10)));
        // Same columns, but one above the other.
        assert!(!rect(0, 0, 10, 10).overlaps(&rect(0, 20, 10, 30)));
    }

    #[test]
    fn touching_edges_is_a_miss() {
        assert!(!rect(0, 0, 10, 10).overlaps(&rect(10, 0, 20, 10)));
        assert!(!rect(0, 0, 10, 10).overlaps(&rect(0, 10, 10, 20)));
    }

    #[test]
    fn overlap_is_symmetric() {
        let a = rect(0, 0, 10, 10);
        let b = rect(9, 9, 12, 12);
        assert_eq!(a.overlaps(&b), b.overlaps(&a));
    }
}
