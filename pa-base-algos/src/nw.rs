//! TODO
//! - Store block of fronts in a single allocation. Update `NwFront` to contain multiple columns as once and be reusable.
//! - Do exponential search for height of the traceback block.
//! - timings
//! - pruning
//! - reuse computed values when doing A*
//! - meet in the middle for traceback
//! - try jemalloc/mimalloc
mod affine;
mod bitpacking;
mod front;

use crate::nw::front::{IRange, JRange, NwFront, NwFronts};
use crate::Domain;
use crate::{exponential_search, Strategy};
use pa_affine_types::*;
use pa_heuristic::*;
use pa_types::*;
use pa_vis_types::*;
use serde::{Deserialize, Serialize};
use std::cmp::{max, min};

use self::affine::AffineNwFrontsTag;
use self::front::NwFrontsTag;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum FrontType {
    Affine,
    Bit(BitFront),
}

// TODO: Fix these names to be the same.
pub use affine::AffineNwFrontsTag as AffineFront;
pub use bitpacking::BitFrontsTag as BitFront;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AstarNwParams {
    /// An optional name for the parameter set.
    #[serde(default)]
    pub name: String,

    /// The domain to compute.
    pub domain: Domain<()>,

    /// Heuristic to use for A* domain.
    pub heuristic: HeuristicParams,

    /// The strategy to use to compute the given domain.
    pub strategy: Strategy,

    /// Compute `block_width` columns at a time, to reduce overhead of metadata
    /// computations.
    pub block_width: I,

    /// The front type to use.
    pub front: FrontType,

    /// When true, `j_range` skips querying `h` when it can assuming consistency.
    #[serde(default)]
    pub sparse_h_calls: bool,
}

impl AstarNwParams {
    /// Build an `AstarStatsAligner` instance from
    pub fn make_aligner(&self, trace: bool) -> Box<dyn Aligner> {
        self.make_aligner_with_visualizer(trace, NoVis)
    }

    /// Build a type-erased aligner object from parameters.
    /// FIXME: Add costmodel support.
    pub fn make_aligner_with_visualizer<V: VisualizerT + 'static>(
        &self,
        trace: bool,
        v: V,
    ) -> Box<dyn Aligner> {
        struct Mapper<V: VisualizerT, F: NwFrontsTag<0>> {
            params: AstarNwParams,
            trace: bool,
            v: V,
            front: F,
        }
        impl<V: VisualizerT + 'static, F: NwFrontsTag<0> + 'static> HeuristicMapper for Mapper<V, F> {
            type R = Box<dyn Aligner>;
            fn call<H: Heuristic + 'static>(self, h: H) -> Box<dyn Aligner> {
                Box::new(NW {
                    cm: AffineCost::unit(),
                    domain: Domain::Astar(h),
                    strategy: self.params.strategy,
                    block_width: self.params.block_width,
                    v: self.v,
                    front: self.front,
                    trace: self.trace,
                    sparse_h: self.params.sparse_h_calls,
                })
            }
        }
        match (self.domain, self.front) {
            (Domain::Astar(()), FrontType::Affine) => self.heuristic.map(Mapper {
                params: self.clone(),
                trace,
                v,
                front: AffineFront,
            }),
            (Domain::Astar(()), FrontType::Bit(front)) => self.heuristic.map(Mapper {
                params: self.clone(),
                trace,
                v,
                front,
            }),
            (d, FrontType::Affine) => Box::new(NW {
                cm: AffineCost::unit(),
                domain: d.into(),
                strategy: self.strategy,
                block_width: self.block_width,
                v,
                front: AffineFront,
                trace,
                sparse_h: self.sparse_h_calls,
            }),
            (d, FrontType::Bit(front)) => Box::new(NW {
                cm: AffineCost::unit(),
                domain: d.into(),
                strategy: self.strategy,
                block_width: self.block_width,
                v,
                front,
                trace,
                sparse_h: self.sparse_h_calls,
            }),
        }
    }
}

/// Needleman-Wunsch aligner.
///
/// NOTE: Heuristics only support unit cost graph for now.
pub struct NW<const N: usize, V: VisualizerT, H: Heuristic, F: NwFrontsTag<N>> {
    /// The cost model to use.
    pub cm: AffineCost<N>,

    /// The domain to compute.
    pub domain: Domain<H>,

    /// The strategy to use to compute the given domain.
    pub strategy: Strategy,

    /// Compute `block_width` columns at a time, to reduce overhead of metadata
    /// computations.
    pub block_width: I,

    /// The visualizer to use.
    pub v: V,

    /// The type of front to use.
    pub front: F,

    /// Whether to return a trace.
    /// `.cost()` always returns cost only, while `.align()` returns a cigar
    /// depending on this.
    pub trace: bool,

    /// When true, `j_range` skips querying `h` when it can assuming consistency.
    pub sparse_h: bool,
}

impl<const N: usize> NW<N, NoVis, NoCost, AffineNwFrontsTag<N>> {
    // TODO: This is only used in tests.
    pub fn new(cm: AffineCost<N>, use_gap_cost_heuristic: bool, exponential_search: bool) -> Self {
        Self {
            cm,
            domain: if use_gap_cost_heuristic {
                Domain::GapGap
            } else {
                Domain::Full
            },
            strategy: if exponential_search {
                // TODO: Make this more general.
                Strategy::band_doubling()
            } else {
                Strategy::None
            },
            // TODO: Make this more general.
            block_width: 32,
            v: NoVis,
            front: AffineNwFrontsTag::<N>,
            trace: true,
            sparse_h: true,
        }
    }
}

impl<const N: usize, V: VisualizerT, H: Heuristic, F: NwFrontsTag<N>> NW<N, V, H, F> {
    pub fn build<'a>(&'a self, a: Seq<'a>, b: Seq<'a>) -> NWInstance<'a, N, V, H, F> {
        use Domain::*;
        NWInstance {
            a,
            b,
            params: self,
            domain: match self.domain {
                Full => Full,
                GapStart => GapStart,
                GapGap => GapGap,
                Astar(h) => Astar(h.build(a, b)),
            },
            v: self.v.build(a, b),
        }
    }

    fn band_doubling_params(
        &self,
        start: crate::DoublingStart,
        a: &[u8],
        b: &[u8],
        nw: &NWInstance<N, V, H, F>,
    ) -> (i32, i32) {
        let (start_f, start_increment) = match start {
            crate::DoublingStart::Zero => (0, 1),
            crate::DoublingStart::Gap => {
                let x = self.cm.gap_cost(Pos(0, 0), Pos::target(a, b));
                (x, x)
            }
            crate::DoublingStart::H0 => (
                nw.domain
                    .h()
                    .expect("DoublingStart::H0 requires an A* domain with heuristic.")
                    .h(Pos(0, 0)),
                1,
            ),
        };
        (start_f, max(start_increment, F::BLOCKSIZE))
    }

    fn cost_or_align(&self, a: Seq, b: Seq, trace: bool) -> (Cost, Option<AffineCigar>) {
        let mut nw = self.build(a, b);
        let (cost, cigar) = match self.strategy {
            Strategy::LocalDoubling => {
                todo!();
                //return nw.align_local_band_doubling();
            }
            Strategy::BandDoubling { start, factor } => {
                let (start_f, start_increment) = self.band_doubling_params(start, a, b, &nw);
                let mut fronts = self.front.new(trace, a, b, &self.cm);
                exponential_search(start_f, start_increment, factor, |s| {
                    nw.align_for_bounded_dist(Some(s), trace, Some(&mut fronts))
                        .map(|x @ (c, _)| (c, x))
                })
                .1
            }
            Strategy::None => {
                // FIXME: Allow single-shot alignment with bounded dist.
                assert!(matches!(self.domain, Domain::Full));
                nw.align_for_bounded_dist(None, trace, None).unwrap()
            }
        };
        nw.v.last_frame::<NoCostI>(cigar.as_ref(), None, None);
        (cost, cigar)
    }

    pub fn cost(&self, a: Seq, b: Seq) -> Cost {
        self.cost_or_align(a, b, false).0
    }

    pub fn align(&self, a: Seq, b: Seq) -> (Cost, Option<AffineCigar>) {
        let (cost, cigar) = self.cost_or_align(a, b, self.trace);
        (cost, cigar)
    }

    pub fn cost_for_bounded_dist(&self, a: Seq, b: Seq, f_max: Cost) -> Option<Cost> {
        self.build(a, b)
            .align_for_bounded_dist(Some(f_max), false, None)
            .map(|c| c.0)
    }

    pub fn align_for_bounded_dist(
        &self,
        a: Seq,
        b: Seq,
        f_max: Cost,
    ) -> Option<(Cost, AffineCigar)> {
        self.build(a, b)
            .align_for_bounded_dist(Some(f_max), true, None)
            .map(|(c, cigar)| (c, cigar.unwrap()))
    }
}

impl<const N: usize, V: VisualizerT, H: Heuristic, F: NwFrontsTag<N>> AffineAligner
    for NW<N, V, H, F>
{
    fn align_affine(&mut self, a: Seq, b: Seq) -> (Cost, Option<AffineCigar>) {
        self.cost_or_align(a, b, true)
    }
}

impl<V: VisualizerT, H: Heuristic, F: NwFrontsTag<0>> Aligner for NW<0, V, H, F> {
    fn align(&mut self, a: Seq, b: Seq) -> (Cost, Option<Cigar>) {
        let (cost, cigar) = NW::align(self, a, b);
        (cost, cigar.map(|c| c.into()))
    }
}

impl<const N: usize, V: VisualizerT, H: Heuristic, F: NwFrontsTag<N>> std::fmt::Debug
    for NW<N, V, H, F>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NW").field("domain", &self.domain).finish()
    }
}

pub struct NWInstance<'a, const N: usize, V: VisualizerT, H: Heuristic, F: NwFrontsTag<N>> {
    // NOTE: `a` and `b` are padded sequences and hence owned.
    pub a: Seq<'a>,
    pub b: Seq<'a>,

    pub params: &'a NW<N, V, H, F>,

    /// The instantiated heuristic to use.
    pub domain: Domain<H::Instance<'a>>,

    /// The instantiated visualizer to use.
    pub v: V::Instance,
}

impl<'a, const N: usize, V: VisualizerT, H: Heuristic, F: NwFrontsTag<N>>
    NWInstance<'a, N, V, H, F>
{
    /// The range of rows `j` to consider for columns `i_range.0 .. i_range.1`, when the cost is bounded by `f_bound`.
    ///
    /// For A*, this also returns the range of rows in column `i_range.0` that are 'fixed', ie have `f <= f_max`.
    /// TODO: We could actually also return such a range in non-A* cases.
    ///
    /// `i_range`: `[start, end)` range of characters of `a` to process. Ends with column `end` of the DP matrix.
    /// Pass `-1..0` for the range of the first column. `prev` is not used.
    /// Pass `i..i+1` to move 1 front, with `prev` the front for column `i`,
    /// Pass `i..i+W` to compute a block of `W` columns `i .. i+W`.
    fn j_range(
        &self,
        i_range: IRange,
        f_max: Option<Cost>,
        prev: &<F::Fronts<'a> as NwFronts<N>>::Front,
    ) -> JRange {
        // Without a bound on the distance, we can only return the full range.
        let Some(f_max) = f_max else {
            return JRange(0, self.b.len() as I);
        };

        // Inclusive start column of the new block.
        let is = i_range.0;
        // Inclusive end column of the new block.
        let ie = i_range.1;

        match &self.domain {
            Domain::Full => JRange(0, self.b.len() as I),
            Domain::GapStart => {
                // range: the max number of diagonals we can move up/down from the start with cost f.
                let range = JRange(
                    -(self.params.cm.max_del_for_cost(f_max) as I),
                    self.params.cm.max_ins_for_cost(f_max) as I,
                );
                // crop
                JRange(
                    max(is + 1 + range.0, 0),
                    min(ie + range.1, self.b.len() as I),
                )
            }
            Domain::GapGap => {
                let d = self.b.len() as I - self.a.len() as I;
                // We subtract the cost needed to bridge the gap from the start to the end.
                let s = f_max
                    - self
                        .params
                        .cm
                        .gap_cost(Pos(0, 0), Pos::target(&self.a, &self.b));
                // Each extra diagonal costs one insertion and one deletion.
                let extra_diagonals =
                    s / (self.params.cm.min_ins_extend + self.params.cm.min_del_extend);
                // NOTE: The range could be reduced slightly further by considering gap open costs.
                let range = JRange(
                    min(d, 0) - extra_diagonals as I,
                    max(d, 0) + extra_diagonals as I,
                );

                // crop
                JRange(
                    max(is + 1 + range.0, 0),
                    min(ie + range.1, self.b.len() as I),
                )
            }
            Domain::Astar(h) => {
                // Get the range of rows with fixed states `f(u) <= f_max`.
                let JRange(fixed_start, fixed_end) = prev
                    .fixed_j_range()
                    .expect("With A* Domain, fixed_j_range should always be set.");

                // Early return for empty range.
                if fixed_start > fixed_end {
                    return JRange(fixed_start, fixed_end);
                }

                // The start of the j_range we will compute for this block is the `fixed_start` of the previous column.
                // The end of the j_range is extrapolated from `fixed_end`.

                // `u` is the bottom most fixed state in prev col.
                let u = Pos(is, fixed_end);
                let gu = if is < 0 { 0 } else { prev.index(fixed_end) };
                // in the end, `v` will be the bottom most state in column
                // i_range.1 that could possibly have `f(v) <= f_max`.
                let mut v = u;

                let mut hint = Default::default();
                // Wrapper to use h with hint.
                let mut h = |pos| {
                    let (h, new_hint) = h.h_with_hint(pos, hint);
                    hint = new_hint;
                    h
                };
                // A lower bound of `f` values estimated from `gu`, valid for states `v` below the diagonal of `u`.
                let mut f = |v: Pos| {
                    assert!(v.1 - u.1 >= v.0 - u.0);
                    gu + self.params.cm.extend_cost(u, v) + h(v)
                };

                // Extend `v` diagonally one column at a time towards `ie`.
                // In each column, find the lowest `v` such that
                // `f(v) = g(v) + h(v) <= gu + extend_cost(u, v) + h(v) <= s`.
                //
                // NOTE: We can not directly go to the last column, since
                // the optimal path could then 'escape' through the bottom.
                // Without further reasoning, we must evaluate `h` at least
                // once per column.

                if self.params.sparse_h {
                    v += Pos(1, 1);
                    // ALG:
                    // First go down by block size.
                    // (This is important; f doesn't work or `v` above the diagonal of `u`.)
                    // Then, go right, until in-scope using exponential steps.
                    // Then down until out-of-scope.
                    // Repeat.
                    // In the end, go up to in-scope.
                    v.1 += self.params.block_width;
                    v.1 = min(v.1, self.b.len() as I);
                    while v.0 <= ie && v.1 < self.b.len() as I {
                        let fv = f(v);
                        if fv <= f_max {
                            // TODO: Make this number larger. Outside the scope,
                            // we can make bigger jumps.
                            v.1 += 1;
                        } else {
                            v.0 += (fv - f_max).div_ceil(2 * self.params.cm.min_del_extend);
                        }
                    }
                    v.0 = ie;
                    loop {
                        // Stop in the edge case where `f(v)` would be invalid (`v.1<0`)
                        // or when the bottom of the grid was reached, in which
                        // case `v` may not be below the diagonal of `u`, and
                        // simply computing everything won't loose much anyway.
                        if v.1 < 0 || v.1 == self.b.len() as I {
                            break;
                        }
                        let fv = f(v);
                        if fv <= f_max {
                            break;
                        } else {
                            v.1 -= (fv - f_max).div_ceil(2 * self.params.cm.min_ins_extend);
                        }
                    }
                } else {
                    while v.0 < ie {
                        // Extend diagonally.
                        v += Pos(1, 1);

                        // Check if cell below is out-of-reach.
                        v.1 += 1;
                        while v.1 <= self.b.len() as I && f(v) <= f_max {
                            v.1 += 1;
                        }
                        v.1 -= 1;
                    }
                }
                JRange(max(fixed_start, 0), min(v.1, self.b.len() as I))
            }
        }
    }

    /// Compute the j_range of `front` `i` with `f(u) <= f_max`.
    /// BUG: This should take into account potential non-consistency of `h`.
    /// In particular, with inexact matches, we can only fix states with `f(u) <= f_max - r`.
    fn fixed_j_range(
        &self,
        i: I,
        f_max: Option<Cost>,
        front: &<F::Fronts<'a> as NwFronts<N>>::Front,
    ) -> Option<JRange> {
        let Domain::Astar(h) = &self.domain else { return None; };
        let Some(f_max) = f_max else { return None; };

        // Wrapper to use h with hint.
        let mut hint = Default::default();
        let mut h = |pos| {
            let (h, new_hint) = h.h_with_hint(pos, hint);
            hint = new_hint;
            h
        };
        let mut f = |j| front.index(j) + h(Pos(i, j));

        // Start: increment the start of the range until f<=f_max is satisfied.
        // End: decrement the end of the range until f<=f_max is satisfied.
        //
        // ALG: Sparse h-calls:
        // Set u = (i, start), and compute f(u).
        // For v = (i, j), (j>start) we have
        // - g(v) >= g(u) - (j - start), by triangle inequality
        // - h(u) <= (j - start) + h(v), by 'column-wise-consistency'
        // => f(u) = g(u) + h(u) <= g(v) + h(v) + 2*(j - start) = f(v) + 2*(j - start)
        // => f(v) >= f(u) - 2*(j - start)
        // We want f(v) <= f_max, so we can stop when f(u) - 2*(j - start) <= f_max, ie
        // j >= start + (f(u) - f_max) / 2
        // Thus, both for increasing `start` and decreasing `end`, we can jump ahead if the difference is too large.
        let mut start = front.j_range().0;
        let mut end = front.j_range().1;
        while start <= end {
            let f = f(start);
            if f <= f_max {
                break;
            }
            start += if self.params.sparse_h {
                (f - f_max).div_ceil(2 * self.params.cm.min_ins_extend)
            } else {
                1
            };
        }

        while end >= start {
            let f = f(end);
            if f <= f_max {
                break;
            }
            end -= if self.params.sparse_h {
                (f - f_max).div_ceil(2 * self.params.cm.min_ins_extend)
            } else {
                1
            };
        }
        Some(JRange(start, end))
    }

    /// Test whether the cost is at most s.
    /// Returns None if no path was found.
    /// It may happen that a path is found, but the cost is larger than s.
    /// In this case no cigar is returned.
    /// TODO: Reuse fronts between iterations.
    fn align_for_bounded_dist(
        &mut self,
        f_max: Option<Cost>,
        trace: bool,
        fronts: Option<&mut F::Fronts<'a>>,
    ) -> Option<(Cost, Option<AffineCigar>)> {
        // Make a local front variable if not passed in.
        let mut local_fronts = if fronts.is_none() {
            Some(
                self.params
                    .front
                    .new(trace, self.a, self.b, &self.params.cm),
            )
        } else {
            None
        };
        let fronts = if let Some(fronts) = fronts {
            fronts
        } else {
            local_fronts.as_mut().unwrap()
        };

        assert!(f_max.unwrap_or(0) >= 0);
        let initial_j_range = self.j_range(IRange::first_col(), f_max, &Default::default());
        if initial_j_range.is_empty() {
            return None;
        }
        eprintln!("Bound: {f_max:?} {initial_j_range:?}");
        fronts.init(initial_j_range);

        self.v.expand_block(
            Pos(0, fronts.last_front().j_range_rounded().0),
            Pos(1, fronts.last_front().j_range_rounded().len()),
            0,
            f_max.unwrap_or(0),
            self.domain.h(),
        );

        for i in (0..self.a.len() as I).step_by(self.params.block_width as _) {
            let i_range = IRange(i, min(i + self.params.block_width, self.a.len() as I));
            let j_range = self.j_range(i_range, f_max, fronts.last_front());
            if j_range.is_empty() {
                return None;
            }
            fronts.compute_next_block(i_range, j_range);
            // Compute the range of fixed states.
            fronts.set_last_front_fixed_j_range(self.fixed_j_range(
                i_range.1,
                f_max,
                fronts.last_front(),
            ));

            self.v.expand_block(
                Pos(i_range.0 + 1, fronts.last_front().j_range_rounded().0),
                Pos(i_range.len(), fronts.last_front().j_range_rounded().len()),
                0,
                f_max.unwrap_or(0),
                self.domain.h(),
            );
            if self.params.strategy == Strategy::None {
                self.v.new_layer(self.domain.h());
            }
        }
        self.v.new_layer(self.domain.h());

        let Some(dist) = fronts.last_front().get(self.b.len() as I) else {
            return None;
        };
        if trace && dist <= f_max.unwrap_or(I::MAX) {
            let cigar = fronts.trace(
                State {
                    i: 0,
                    j: 0,
                    layer: None,
                },
                State {
                    i: self.a.len() as I,
                    j: self.b.len() as I,
                    layer: None,
                },
            );
            Some((dist, Some(cigar)))
        } else {
            Some((dist, None))
        }
    }

    /// FIXME: This is unmaintained at the moment.
    #[cfg(any())]
    pub fn align_local_band_doubling<'b>(&mut self) -> (Cost, AffineCigar) {
        assert!(
            !H::IS_DEFAULT,
            "Local doubling needs a heuristic. Use -H zero to disable."
        );

        let h0 = self.domain.h().unwrap().h(Pos(0, 0));
        let mut fronts = NwFronts::new(
            &self.a,
            &self.b,
            &self.params.cm,
            self.j_range(IRange::first_col(), Some(h0), &NwFronts::default()),
        );

        // Front i has been computed up to this f.
        let mut f_max = vec![h0];
        // Each time a front is grown, it grows to the least multiple of delta that is large enough.
        // Delta doubles after each grow.
        const DELTA_0: Cost = 2;
        let mut f_delta = vec![2];

        // The value of f at the tip. When going to the next front, this is
        // incremented until the range is non-empty.
        let mut f_tip = self.domain.h().unwrap().h(Pos(0, 0));

        let mut i = 0;
        // This is a for loop over `i`, but once `i` reaches `a.len()`, the last
        // front is grown instead of increasing `i`.
        loop {
            if i < self.a.len() as I {
                // Add a new front.
                let mut range;
                loop {
                    // println!("{i} => {f_tip} try");
                    range = self.j_range(IRange(i, i + 1), Some(f_tip), &fronts.fronts[i]);
                    if !range.is_empty() {
                        break;
                    }
                    f_tip += 1;
                }
                i += 1;
                f_max.push(f_tip);
                f_delta.push(DELTA_0);
                fronts.fronts.push_default_front(range.into());
            } else {
                // Only grow the last front.
                let delta = &mut f_delta[i as usize];
                // print!("Double last front from {} by {delta}", f_max[i as usize]);
                f_max[i as usize] = (f_max[i as usize] / *delta + 1) * *delta;
                // println!("to {}", f_max[i as usize]);
                *delta *= 2;
            }

            // Double previous front sizes as long as their f_max is not large enough.
            let mut start_i = i as usize;
            while start_i > 1 && f_max[start_i - 1] < f_max[start_i] {
                // Check if (after pruning) the range for start_i needs to grow at all.
                start_i -= 1;
                {
                    let front = &fronts.fronts[start_i as I];
                    let js = *front.range().start();
                    let je = *front.range().end();
                    // println!(
                    //     "Row {js}\t g {} + h {} > f_next {} (f_cur {})",
                    //     front.m()[js as Idx],
                    //     h.h(Pos(start_i as I - 1, js as I - 1)),
                    //     f_max[start_i + 1],
                    //     f_max[start_i]
                    // );
                    // println!(
                    //     "Row {je}\t g {} + h {} > f_next {} (f_cur {})",
                    //     front.m()[je as Idx],
                    //     h.h(Pos(start_i as I - 1, je as I - 1)),
                    //     f_max[start_i + 1],
                    //     f_max[start_i]
                    // );
                    // FIXME: Generalize to more layers.
                    // NOTE: -1's are to correct for sequence padding.
                    // NOTE: equality isn't good enough: in that case there
                    // could be adjacent states that also have equality.
                    if front.m()[js as I]
                        + self
                            .domain
                            .h()
                            .unwrap()
                            .h(Pos(start_i as I - 1, js as I - 1))
                        > f_max[start_i + 1]
                        && front.m()[je as I]
                            + self
                                .domain
                                .h()
                                .unwrap()
                                .h(Pos(start_i as I - 1, je as I - 1))
                            > f_max[start_i + 1]
                    {
                        start_i += 1;
                        // println!(
                        //     "Stop. Col {} is last to reuse. Col {start_i} is recomputed",
                        //     start_i - 1
                        // );
                        break;
                    }
                }

                let before = f_max[start_i];
                let delta = &mut f_delta[start_i];
                f_max[start_i] = f_max[start_i + 1].next_multiple_of(*delta);
                // println!("{start_i} => {before} -> {} \t ({delta})", f_max[start_i]);
                assert!(
                    f_max[start_i] >= f_max[start_i + 1],
                    "Doubling not enough!? From {before} to {} by {delta} target {}",
                    f_max[start_i],
                    f_max[start_i + 1]
                );
                *delta *= 2;
            }

            if start_i > 1 {
                // for j in fronts[start_i as Idx - 1].range().clone() {
                //     let i = start_i - 1;
                //     println!(
                //         "row {j} \t g-prev {:10} \t h-new {}",
                //         fronts[i as Idx].m().get(j).unwrap_or(&Cost::MAX),
                //         h.h(Pos(i as I - 1, j as I - 1))
                //     )
                // }
            }

            // Recompute all fronts from start_i upwards.
            for i in start_i as I..=i {
                let range = self.j_range(
                    IRange(i - 1, i),
                    Some(f_max[i as usize]),
                    &fronts.fronts[i - 1],
                );
                let prev_range = fronts.fronts[i as I].range().clone();
                let new_range = min(range.0, *prev_range.start())..=max(range.1, *prev_range.end());
                // println!(
                //     "Compute {i} for {} => {new_range:?} (prev {prev_range:?})",
                //     f_max[i as usize],
                // );
                // if range.is_empty() || true {
                //     for j in new_range.clone() {
                //         println!(
                //             "row {j} \t g-prev {:10} \t h-new {}",
                //             fronts[i as Idx].m().get(j).unwrap_or(&Cost::MAX),
                //             h.h(Pos(i as I - 1, j as I - 1))
                //         )
                //     }
                // }
                assert!(!new_range.is_empty());
                fronts.update_fronts(i..i + 1, new_range.clone(), |pos, g| {
                    self.v.expand(pos, g, f_max[i as usize], self.domain.h())
                });

                // for j in new_range.clone() {
                //     println!(
                //         "row {j} \t g-prev {:10} \t h-new {}",
                //         fronts[i as Idx].m().get(j).unwrap_or(&Cost::MAX),
                //         h.h(Pos(i as I - 1, j as I - 1))
                //     )
                // }

                // Prune matches
                if self
                    .domain
                    .h()
                    .unwrap()
                    .is_seed_start_or_end(Pos(i as I - 1, 0))
                {
                    let hint = self
                        .domain
                        .h()
                        .unwrap()
                        .h_with_hint(Pos(i as I - 1, *new_range.start() as I), Default::default())
                        .1;
                    for j in new_range {
                        self.domain
                            .h_mut()
                            .unwrap()
                            .prune(Pos(i as I - 1, j as I), hint);
                    }
                }

                self.v.new_layer(Some(self.domain.h().unwrap()));
            }

            if i == self.a.len() as I
                && fronts.fronts[self.a.len() as I]
                    .range()
                    .contains(&(self.b.len() as I))
            {
                break;
            }
        } // end loop

        let dist = *fronts.fronts[self.a.len() as I]
            .m()
            .get(self.b.len() as I)
            .unwrap();
        let cigar = self.trace(
            &fronts,
            State {
                i: 1,
                j: 1,
                layer: None,
            },
            State {
                i: self.a.len() as I,
                j: self.b.len() as I,
                layer: None,
            },
            Direction::Forward,
        );
        self.v
            .last_frame(Some(&cigar), None, Some(self.domain.h().unwrap()));
        (dist, cigar)
    }
}

#[cfg(test)]
mod test {
    use pa_affine_types::AffineCost;
    use pa_heuristic::{MatchConfig, Pruning, GCSH};
    use pa_vis_types::NoVis;

    use crate::{Domain, Strategy};

    use super::{BitFront, NW};

    #[test]
    fn nw() {
        let (a, b) =
            pa_generate::generate_model(10000, 0.1, pa_generate::ErrorModel::Uniform, 31415);
        let d = NW {
            cm: AffineCost::unit(),
            strategy: Strategy::band_doubling(),
            domain: Domain::Astar(GCSH::new(MatchConfig::inexact(15), Pruning::start())),
            block_width: 256,
            v: NoVis,
            front: BitFront {
                sparse: true,
                simd: true,
            },
            trace: true,
            sparse_h: true,
        }
        .align(&a, &b)
        .0;
        let d2 = triple_accel::levenshtein_exp(&a, &b) as _;
        assert_eq!(d, d2);
    }
}
