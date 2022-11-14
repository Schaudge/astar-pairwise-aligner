use std::marker::PhantomData;

use bio::alignment::distance::simd::levenshtein;

use crate::{
    aligners::{astar::AStar, cigar::test::verify_cigar, Aligner},
    cost_model::LinearCost,
    heuristic::{Heuristic, Pruning, CSH},
    matches::MatchConfig,
    prelude::{BruteForceContour, HintContours},
    visualizer::*,
};

mod contours;

fn test_input(a: &[u8], b: &[u8], dt: bool, h: impl Heuristic) {
    let mut aligner = AStar {
        greedy_edge_matching: true,
        diagonal_transition: dt,
        h,
        //v: NoVisualizer,
        v: Visualizer::new(Config::new(VisualizerStyle::Test), a, b),
    };
    let (d, cigar) = aligner.align(a, b);
    verify_cigar(&LinearCost::new_unit(), a, b, &cigar);
    let dist = levenshtein(a, b);
    assert_eq!(d, dist);
}

/// thread 'tests::bug_in_csh_contours' panicked at 'assertion failed: new_layer <= v', src/contour/hint_contours.rs:413:17
/// This tests that hint contours only remove contours when at least `max_len + shift - 1` layers have shifted down by `shift`.
/// Before it only checked for at least `max_len` layers, which is wrong.
#[test]
fn hint_contours_overly_greedy_shift_1() {
    let h = CSH {
        match_config: MatchConfig::new(3, 1),
        pruning: Pruning::new(true),
        use_gap_cost: true,
        c: PhantomData::<HintContours<BruteForceContour>>::default(),
    };

    let a = "CCCGTCGTCCCTCAAACTTGGAACCCCATCGCAAATCACCCCACAGGTAACGTCATAACTACCGCATGGTACGGTACCCCTTCTGCGATAGAGATGGTAGTAGCCGATAGGCCACCCTGGGAACACTATGTCACCCTGGTGGTAACCGTCGGGTCAGAAATAGGAGAACATACGGTGGACCGCTAA".as_bytes();
    let b = "CCCGTCGTACCTCTAAACTTGGAACCCACATCGCAAATCACCCCACAGGTAACGTCATAACTACCGCATGGTTCGGGTACCCCTTCGTGCGATAGAGATGGTAGTAGCCGATAGGCCACCCTGGGAACACTATGTCACCCTGGTGGTAACCGTCGGGTCAGAAATAGGAGTACATACGGTGGACCG".as_bytes();
    test_input(a, b, false, h);
}

#[test]
fn hint_contours_overly_greedy_shift_2() {
    let h = CSH {
        match_config: MatchConfig::new(4, 1),
        pruning: Pruning::new(true),
        use_gap_cost: true,
        c: PhantomData::<HintContours<BruteForceContour>>::default(),
    };

    let a = "ATATATATTAGCGGGCATTCGCCGACCTGGAAGTGCCAGGCCATTTCGTAGCAGTAGGTCCTCACCAAGGCCAGGCAAGTCGGTAGTAAAAT".as_bytes();
    let b = "ATATATATTAAGCTGGCCTATTCGCGACCTGCGAAGGGGCCAGGCATTTCCTATCAGTAGGTCCCTCACCAAAGCCAGGT"
        .as_bytes();
    test_input(a, b, false, h);
}

#[test]
fn hint_contours_overly_greedy_shift_3() {
    let h = CSH {
        match_config: MatchConfig::new(3, 1),
        pruning: Pruning::new(true),
        use_gap_cost: true,
        c: PhantomData::<HintContours<BruteForceContour>>::default(),
    };

    let a = "TTCCGACACTAGCTGTCAGCCTTATAACTCATGCCCTAGTATCAACAGGCC".as_bytes();
    let b = "TTTCCGACCACTAGCTAACTCATGTCCCAGTTCAACAGGCCGTGGGAC".as_bytes();
    test_input(a, b, false, h);
}

/// Since CSH is not consistent, f may go up while extending a greedy match.
/// This means that we can not freely extend and prune expanded states in DT-A*.
///
/// Fixed by a complete rewrite of DT-A*:
/// - We now store normal Pos in the priority queue instead of DtPos.
/// - Like normal A*, extending is done before pushing a state onto the priority queue.
#[test]
fn csh_dt_inconsistent_greedy_1() {
    let h = CSH {
        match_config: MatchConfig::new(3, 1),
        pruning: Pruning::new(true),
        use_gap_cost: false,
        c: PhantomData::<HintContours<BruteForceContour>>::default(),
    };

    let a = "GCCGCGCGCGCAGCCGCGCGCGCGCGCGCGCCGG".as_bytes();
    let b = "GCGCCAGCGCGCGCGGGCCGCCGGCGCGCGCGCT".as_bytes();
    test_input(a, b, true, h);
}

#[test]
fn csh_dt_greedy_inconsistent_2() {
    let h = CSH {
        match_config: MatchConfig::new(3, 1),
        pruning: Pruning::new(true),
        use_gap_cost: false,
        c: PhantomData::<HintContours<BruteForceContour>>::default(),
    };

    let a = "TCTCTCTCTCTG".as_bytes();
    let b = "GTCTCTCTTCTG".as_bytes();
    test_input(a, b, true, h);
}
