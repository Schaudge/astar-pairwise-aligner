//! This generates the visualizations used in the blogpost on linear memory WFA.

#[cfg(not(feature = "sdl2"))]
fn main() {}

#[cfg(feature = "sdl2")]
fn main() {
    use std::{path::PathBuf, time::Duration};

    use astar_pairwise_aligner::{
        aligners::{
            diagonal_transition::{DiagonalTransition, GapCostHeuristic, PathTracingMethod},
            Aligner,
        },
        canvas::{BLUE, RED},
        prelude::*,
        visualizer::{Gradient, Visualizer, When},
    };
    let a = b"CACTGCAATCGGGAGTCAGTTCAGTAACAAGCGTACGACGCCGATACATGCTACGATCGA";
    let b = b"CATCTGCTCTCTGAGTCAGTGCAGTAACAGCGTACG";

    let cm = LinearCost::new_unit();
    let mut config = visualizer::Config::default();
    config.draw = When::All;
    config.save = When::None;
    config.save_last = true;
    config.delay = Duration::from_secs_f32(0.0001);
    config.cell_size = 16;
    config.style.bg_color = (255, 255, 255, 128);
    config.style.expanded = Gradient::TurboGradient(0.25..0.90);
    config.style.path_width = Some(7);
    config.style.tree = Some((64, 64, 64, 0));
    config.style.tree_width = 3;
    config.draw_old_on_top = false;
    config.layer_drawing = false;
    let vis = |a, b, mut config: visualizer::Config, name: &str| {
        config.filepath = PathBuf::from("imgs/path-tracing/").join(name);
        Visualizer::new(config, a, b)
    };

    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "forward-greedy"),
        );
        dt.align(a, b);
    }
    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "backward-greedy"),
        );
        dt.path_tracing_method = PathTracingMethod::ReverseGreedy;
        dt.align(a, b);
    }

    config.style.expanded = Gradient::Fixed((200, 200, 200, 0));
    config.style.extended = Some((230, 230, 230, 0));

    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "forward-greedy-grey"),
        );
        dt.align(a, b);
    }

    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "backward-greedy-grey"),
        );
        dt.path_tracing_method = PathTracingMethod::ReverseGreedy;
        dt.align(a, b);
    }

    config.style.tree_substitution = Some(RED);

    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "forward-greedy-subs"),
        );
        dt.align(a, b);
    }
    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "backward-greedy-subs"),
        );
        dt.path_tracing_method = PathTracingMethod::ReverseGreedy;
        dt.align(a, b);
    }
    {
        let b = b"AXBDBBC";
        let a = b"ABDBBYDC";
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "detail"),
        );
        dt.path_tracing_method = PathTracingMethod::ReverseGreedy;
        dt.align(a, b);
    }
    {
        let a = b"CCGGGGTGCTCG";
        let b = b"GTGCCCGTGGGTG";
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "detail-tricky"),
        );
        dt.align(a, b);
    }

    {
        let a = b"CTTGTGGATCTTAAGGGCATCATAGTGGATCTCGTTGACTTGTGGATCTTAGCTGGATCATAGTGGTTCTTAGGGAGTCTCAAATGGATCTTAGTGGGTCTTAGTGGAAT";
        let b = b"CTTAGTGGATCTAGTGGGACTCTAGTGAATCTTAGTGGCATCTAGCTGATTCGACTAGTGGA";

        {
            let mut dt = DiagonalTransition::new(
                cm.clone(),
                GapCostHeuristic::Disable,
                NoCost,
                false,
                vis(a, b, config.clone(), "repeats"),
            );
            dt.align(a, b);
        }

        config.style.tree_match = Some((160, 160, 160, 0));
        {
            let mut dt = DiagonalTransition::new(
                cm.clone(),
                GapCostHeuristic::Disable,
                NoCost,
                false,
                vis(a, b, config.clone(), "repeats-no-matches"),
            );
            dt.align(a, b);
        }

        config.style.tree = Some((160, 160, 160, 0));
        {
            let mut dt = DiagonalTransition::new(
                cm.clone(),
                GapCostHeuristic::Disable,
                NoCost,
                false,
                vis(a, b, config.clone(), "repeats-subs"),
            );
            dt.align(a, b);
        }

        config.style.tree_fr_only = true;
        {
            let mut dt = DiagonalTransition::new(
                cm.clone(),
                GapCostHeuristic::Disable,
                NoCost,
                false,
                vis(a, b, config.clone(), "repeats-active"),
            );
            dt.align(a, b);
        }

        {
            config.style.tree_direction_change = Some(BLUE);
            let mut dt = DiagonalTransition::new(
                cm.clone(),
                GapCostHeuristic::Disable,
                NoCost,
                false,
                vis(a, b, config.clone(), "repeats-fixed"),
            );
            dt.align(a, b);
        }
    }
    config.style.expanded = Gradient::Fixed((200, 200, 200, 0));
    config.style.extended = Some((230, 230, 230, 0));
    config.style.tree_substitution = Some(RED);
    config.style.tree = Some((160, 160, 160, 0));
    config.style.tree_fr_only = true;
    config.style.tree_direction_change = Some(BLUE);

    {
        let mut dt = DiagonalTransition::new(
            cm.clone(),
            GapCostHeuristic::Disable,
            NoCost,
            false,
            vis(a, b, config.clone(), "simple-final"),
        );
        dt.align(a, b);
    }
}
