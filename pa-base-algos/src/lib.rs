#![feature(let_chains, step_trait)]

use pa_types::Cost;
use std::cmp::max;

mod algorithm_args;
mod cigar;
mod cost_model;
mod edit_graph;
mod front;

mod dt;
mod nw;

#[cfg(test)]
mod tests;

/// Find the cost using exponential search based on `cost_assuming_bounded_dist`.
fn exponential_search<T>(
    s0: Cost,
    factor: f32,
    mut f: impl FnMut(Cost) -> Option<(Cost, T)>,
) -> (Cost, T) {
    let mut s = s0;
    // TODO: Fix the potential infinite loop here.
    loop {
        if let Some((cost,t)) = f(s) && cost <= s{
            return (cost, t);
        }
        s = max((factor * s as f32).ceil() as Cost, 1);
    }
}