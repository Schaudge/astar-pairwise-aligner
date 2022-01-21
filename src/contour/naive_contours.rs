use itertools::Itertools;

use crate::prelude::*;

/// A Contours implementation based on Contour layers with value queries in O(log(r)^2).
///
/// A contour x may contain points p that are actually in contour x+1, but only have value x.
/// This happens e.g. when a length 1 arrow is shadowed by a length 2 arrow.
/// It would be wrong to store p in the x+1 contour, because pruning other
/// points in x+1 could make p dominant there, which is wrong.
/// Hence, we store p in the x contour. This implies that sometimes we add
/// points to a contour that are larger than other points it already contains.
#[derive(Default, Debug)]
pub struct NaiveContours<C: Contour> {
    contours: Vec<C>,
    // TODO: Do not use vectors inside a hashmap.
    arrows: HashMap<Pos, Vec<Arrow>>,
    // TODO: This should have units in the transformed domain instead.
    max_len: I,
    prune_stats: PruneStats,
}

#[derive(Default, Debug)]
struct PruneStats {
    // Total number of prunes we do.
    prunes: usize,
    // Number of times f is evaluated.
    checked: usize,
    // Number of times f evaluates to true/false.
    checked_true: usize,
    checked_false: usize,
    // Total number of layers processed.
    contours: usize,

    // Number of times we stop pruning early.
    no_change: usize,
    shift_layers: usize,
}

impl<C: Contour> NaiveContours<C> {
    /// Get the value of the given position.
    /// It can be that a contour is completely empty, and skipped by length>1 arrows.
    /// In that case, normal binary search would give a wrong answer.
    /// Thus, we always have to check multiple contours.
    // TODO: Is max_len a cost or I here?
    fn value_in_slice(contours: &[C], q: Pos, max_len: I) -> Cost {
        // q is always contained in layer 0.
        let mut left = 1;
        let mut right = contours.len();
        let mut size = right;
        while left < right {
            let mid = left + size / 2;
            let mut found = false;
            if USE_SHADOW_POINTS {
                found = mid < contours.len() && contours[mid].contains(q);
            } else {
                for c in mid..mid + max_len as usize {
                    if c >= contours.len() {
                        break;
                    }
                    let contains = contours[c].contains(q);
                    if contains {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                left = mid + 1;
            } else {
                right = mid;
            }
            size = right - left;
        }
        left as Cost - 1
    }
}

impl<C: Contour> Contours for NaiveContours<C> {
    fn new(arrows: impl IntoIterator<Item = Arrow>, max_len: I) -> Self {
        let mut this = NaiveContours {
            contours: vec![C::default()],
            arrows: HashMap::default(),
            max_len,
            prune_stats: Default::default(),
        };
        this.contours[0].push(Pos(I::MAX, I::MAX));
        // Loop over all arrows from a given positions.
        for (start, pos_arrows) in &arrows.into_iter().group_by(|a| a.start) {
            let mut v = 0;
            this.arrows.insert(start, pos_arrows.collect());
            for a in &this.arrows[&start] {
                assert_eq!((a.end.0 - a.start.0) + (a.end.1 - a.start.1), 2 * max_len);
                v = max(v, this.value(a.end) + a.len);
            }
            assert!(v > 0);
            if this.contours.len() as Cost <= v {
                this.contours
                    .resize_with(v as usize + 1, || C::with_max_len(max_len));
            }
            ////println!("Push {} to layer {}", start, v);
            this.contours[v as usize].push(start);
            if USE_SHADOW_POINTS {
                while v > 0 && !this.contours[v as usize - 1].contains(start) {
                    v -= 1;
                    this.contours[v as usize].push(start);
                }
            }
        }
        this
    }

    fn value(&self, q: Pos) -> Cost {
        Self::value_in_slice(&self.contours, q, self.max_len)
        ////println!("Value of {} : {}", q, v);
    }

    // The layer for the parent node.
    type Hint = ();

    fn prune(&mut self, p: Pos) -> bool {
        if self.arrows.remove(&p).is_none() {
            // This position was already pruned or never needed pruning.
            return false;
        }

        // Work contour by contour.
        // 1. Remove p from it's first contour.
        let mut v = self.value(p);
        //for (i, c) in self.contours.iter().enumerate().rev() {
        //println!("{}: {:?}", i, c);
        //}

        // Prune the current point, and also any other lazily pruned points that become dominant.
        if !self.contours[v as usize].prune_filter(&mut |pos| !self.arrows.contains_key(&pos)) {
            //println!("SKIP");
            return false;
        }
        if USE_SHADOW_POINTS {
            // Also remove the point from other contours where it is dominant.
            let mut shadow_v = v - 1;

            while self.contours[shadow_v as usize].is_dominant(p) {
                self.contours[shadow_v as usize].prune(p);
                shadow_v -= 1;
            }
        }

        self.prune_stats.prunes += 1;
        //println!("PRUNE {} at LAYER {}", p, v);

        // Loop over the dominant matches in the next layer, and repeatedly prune while needed.
        let mut last_change = v;
        let mut num_emptied = 0;
        let mut previous_shift = None;
        loop {
            v += 1;
            if v >= self.contours.len() as Cost {
                break;
            }
            self.prune_stats.contours += 1;
            //println!("layer {}", v);
            //println!("{}: {:?}", v, self.contours[v]);
            //println!("{}: {:?}", v - 1, self.contours[v - 1]);
            let (up_to_v, current) = {
                let (up_to_v, from_v) = self.contours.as_mut_slice().split_at_mut(v as usize);
                (up_to_v, &mut from_v[0])
            };
            // We need to make a reference here to help rust understand we borrow disjoint parts of self.
            let mut current_shift = None;
            let mut layer_best_start_val = 0;
            let changes = current.prune_filter(&mut |pos| -> bool {
                // This function decides whether the point pos from contour v
                // needs to be pruned from it.  For this, we (re)compute the
                // value at pos and if it's < v, we push is to the new contour
                // of its value.
                self.prune_stats.checked += 1;
                //println!("f: {}", pos);
                let pos_arrows = match self.arrows.get(&pos) {
                    Some(arrows) => arrows,
                    None => {
                        //println!("f: Prune {} no arrows left", pos);
                        current_shift = Some(Cost::MAX);
                        // If no arrows left for this position, prune it without propagating.
                        self.prune_stats.checked_true += 1;
                        return true;
                    }
                };
                assert!(!pos_arrows.is_empty());
                let mut best_start_val = 0;
                for arrow in pos_arrows {
                    // Find the value at end_val via a backwards search.
                    let mut end_val = v - arrow.len;
                    while !up_to_v[end_val as usize].contains(arrow.end) {
                        end_val -= 1;

                        // No need to continue when this value isn't going to be optimal anyway.
                        if end_val + arrow.len <= best_start_val {
                            break;
                        }

                        if FAST_ASSUMPTIONS {
                            // We know that max_new_val will be within [v-max_len, v].
                            // Thus, value(arrow.end) will be in [v-max_len-arrow.len, v-arrow.len].
                            // For simplicity, we skip this check.
                            if end_val + self.max_len == v - arrow.len {
                                break;
                            }
                        }
                    }

                    let start_val = end_val + arrow.len;
                    best_start_val = max(best_start_val, start_val);
                    layer_best_start_val = max(layer_best_start_val, start_val);
                }
                // Value v is still up to date. No need to loop over the remaining arrows starting here.
                if best_start_val >= v {
                    //println!("f: {} keeps value {}", pos, best_start_val);
                    self.prune_stats.checked_false += 1;
                    current_shift = Some(Cost::MAX);
                    return false;
                }

                //println!("f: {} new value {}", pos, max_new_val);
                // NOTE: This assertion does not always hold. In particular,
                // when the Contour implementation is lazy about pruning
                // non-dominant points, it may happen that e.g. a value 8 contour contains points with value 7.
                // After removing a match of length max_len=2, this would drop to 5, which is less than 8 - 2.
                // assert!(v - max_len <= max_new_val && max_new_val <= v,);

                // Either no arrows left (position already pruned), or none of its arrows yields value v.
                // println!(
                //     "f: Push {} to {} shift {:?}",
                //     pos, best_start_val, current_shift
                // );
                up_to_v[best_start_val as usize].push(pos);
                if USE_SHADOW_POINTS {
                    let mut v = best_start_val;
                    while v > 0 && !up_to_v[v as usize - 1].contains(pos) {
                        v -= 1;
                        up_to_v[v as usize].push(pos);
                    }
                }
                if current_shift.is_none() {
                    current_shift = Some(v - best_start_val);
                } else if current_shift.unwrap() != v - best_start_val {
                    current_shift = Some(Cost::MAX);
                }
                self.prune_stats.checked_true += 1;
                true
            });
            if changes {
                last_change = v;
            }
            //println!("{}: {:?}", v, self.contours[v]);
            //println!("{}: {:?}", v - 1, self.contours[v - 1]);

            if v >= last_change + self.max_len as Cost {
                ////println!("Last change at {}, stopping at {}", last_change, v);
                // No further changes can happen.
                self.prune_stats.no_change += 1;
                break;
            }

            //println!(
            //"emptied {:?} shift {:?} last_change {:?}",
            //emptied_shift, shift_to, last_change
            //);
            if self.contours[v as usize].len() == 0
                && (current_shift.is_none() || current_shift.unwrap() != Cost::MAX)
            {
                if previous_shift.is_none()
                    || current_shift.is_none()
                    || previous_shift == current_shift
                {
                    num_emptied += 1;
                    if previous_shift.is_none() {
                        previous_shift = current_shift;
                    }
                }
                //println!("Num emptied to {} shift {:?}", num_emptied, emptied_shift);
            } else {
                num_emptied = 0;
                previous_shift = None;
                //println!("Num emptied reset");
            }
            assert!(
                // 0 happens when the layer was already empty.
                layer_best_start_val == 0 || layer_best_start_val >= v - self.max_len,
                "Pruning {} now layer {} new max {} drops more than {}.\nlast_change: {}, shift_to {:?}, layer size: {}",
                p,
                v,
                layer_best_start_val,
                self.max_len,
                last_change, current_shift, self.contours[v as usize].len()
            );

            if num_emptied >= self.max_len {
                //println!("Emptied {}, stopping at {}", num_emptied, v);
                // Shift all other contours one down.
                if let Some(previous_shift) = previous_shift {
                    self.prune_stats.shift_layers += 1;

                    for _ in 0..previous_shift {
                        //println!("Delete layer {} of len {}", v, self.contours[v].len());
                        assert!(self.contours[v as usize].len() == 0);
                        // TODO: Instead of removing contours, keep a Fenwick Tree that counts the number of removed layers.
                        self.contours.remove(v as usize);
                        v -= 1;
                    }
                    break;
                }
            }
        }
        while let Some(c) = self.contours.last() {
            if c.len() == 0 {
                self.contours.pop();
            } else {
                break;
            }
        }
        for l in (0..8).rev() {
            if self.contours.len() > l {
                ////println!("Contour {}: {:?}", l, self.contours[l]);
            }
        }
        // for (i, c) in self.contours.iter().enumerate().rev() {
        //     //println!("{}: {:?}", i, c);
        // }
        true
    }

    fn print_stats(&self) {
        return;
        println!("----------------------------");
        let num = self.contours.len();
        let mut total_len = 0;
        let mut total_dom = 0;
        for c in &self.contours {
            total_len += c.len();
            total_dom += c.num_dominant();
        }
        println!("#contours             {}", num);
        println!("avg size              {}", total_len as f32 / num as f32);
        println!("avg domn              {}", total_dom as f32 / num as f32);

        let PruneStats {
            prunes,
            checked,
            checked_true,
            checked_false,
            contours,
            no_change,
            shift_layers,
        }: PruneStats = self.prune_stats;

        if prunes == 0 {
            return;
        }

        println!("#prunes               {}", prunes);
        println!("contours per prune    {}", contours as f32 / prunes as f32);
        println!("#checks               {}", checked);
        println!("checked per prune     {}", checked as f32 / prunes as f32);
        println!(
            "checked true per p    {}",
            checked_true as f32 / prunes as f32
        );
        println!(
            "checked false per p   {}",
            checked_false as f32 / prunes as f32
        );
        println!("Stop count: no change    {}", no_change);
        println!("Stop count: shift layers {}", shift_layers);
        // println!(
        //     "Stop layer: no change    {}",
        //     sum_no_change_layers as f32 / no_change as f32
        // );
        // println!(
        //     "Stop layer: shift layers {}",
        //     sum_shift_stop_layers as f32 / shift_layers as f32
        // );
        // println!(
        //     "Rem. layer: no change    {}",
        //     sum_no_change_layers_remaining as f32 / no_change as f32
        // );
        // println!(
        //     "Rem. layer: shift layers {}",
        //     sum_shift_stop_layers_remaining as f32 / shift_layers as f32
        // );
        println!("----------------------------");
    }
}