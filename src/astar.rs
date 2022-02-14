#[allow(unused_imports)]
use crate::diagonal_map::{DiagonalMap, DiagonalMapTrait};
use crate::prelude::*;
use crate::scored::MinScored;

#[derive(Clone, Copy, Debug)]
enum Status {
    Unvisited,
    Explored,
    Expanded,
}
use Status::*;

#[derive(Clone, Copy, Debug)]
struct State<Parent, Hint> {
    status: Status,
    g: Cost,
    parent: Parent,
    hint: Hint,
}

impl<Parent: Default, Hint: Default> Default for State<Parent, Hint> {
    fn default() -> Self {
        Self {
            status: Unvisited,
            g: Cost::MAX,
            parent: Parent::default(),
            hint: Hint::default(),
        }
    }
}

#[derive(Serialize, Default, Clone)]
pub struct AStarStats<Pos> {
    pub expanded: usize,
    pub explored: usize,
    pub skipped_explored: usize,
    /// Number of times an already expanded node was expanded again with a lower value of f.
    pub double_expanded: usize,
    /// Number of times a node was popped and found to have an outdated value of h after pruning.
    pub retries: usize,
    /// Number of times a prune is propagated to the priority queue.
    pub pq_shifts: usize,
    /// Number of states allocated in the DiagonalMap
    pub diagonalmap_capacity: usize,
    #[serde(skip_serializing)]
    pub explored_states: Vec<Pos>,
    #[serde(skip_serializing)]
    pub expanded_states: Vec<Pos>,
}

// h: heuristic = lower bound on cost from node to end
// g: computed cost to reach node from the start
// f: g+h
// TODO: Inline on_expand and on_explore functions by direct calls to h.
pub fn astar<'a, H>(
    graph: &AlignmentGraph,
    start: Pos,
    target: Pos,
    h: &mut H,
) -> Option<(Cost, Vec<Pos>, AStarStats<Pos>)>
where
    H: HeuristicInstance<'a, Pos = Pos>,
{
    let mut stats = AStarStats {
        expanded: 0,
        explored: 0,
        skipped_explored: 0,
        double_expanded: 0,
        retries: 0,
        pq_shifts: 0,
        explored_states: Vec::default(),
        expanded_states: Vec::default(),
        diagonalmap_capacity: 0,
    };

    // f -> pos
    let mut queue = heap::Heap::<Cost>::default();
    // When > 0, queue[x] corresponds to f=x+offset.
    // Increasing the offset implicitly shifts all elements of the queue up.
    let mut queue_offset: Cost = 0;
    // An upper bound on the queue_offset, to make sure indices never become negative.
    let max_queue_offset = if REDUCE_RETRIES {
        h.root_potential()
    } else {
        0
    };

    //let mut states = DiagonalMap::<State<Parent, H::Hint>>::new(target);
    let mut states = HashMap::<Pos, State<Parent, H::Hint>>::new(target);

    {
        let (hroot, hint) = h.h_with_hint(start, H::Hint::default());
        queue.push(MinScored(
            hroot + (max_queue_offset - queue_offset),
            start,
            0,
        ));
        states.insert(
            start,
            State {
                status: Explored,
                g: 0,
                parent: Default::default(),
                hint,
            },
        );
    }

    while let Some(MinScored(queue_f, mut pos, queue_g)) = queue.pop() {
        let queue_f = queue_f + queue_offset - max_queue_offset;
        // This lookup can be unwrapped without fear of panic since the node was necessarily scored
        // before adding it to `visit_next`.
        //let g = gs[pos];
        let state = &mut states[pos];

        if queue_g > state.g {
            continue;
        }

        assert!(queue_g == state.g);

        let g = state.g;
        let hint = state.hint;

        // If the heuristic value is outdated, skip the node and re-push it with the updated value.
        if RETRY_OUDATED_HEURISTIC_VALUE {
            let (current_h, new_hint) = h.h_with_hint(pos, state.hint);
            state.hint = new_hint;
            let current_f = g + current_h;
            assert!(
                current_f >= queue_f,
                "Current_f {current_f} smaller than queue_f {queue_f}!"
            );
            if current_f > queue_f {
                stats.retries += 1;
                queue.push(MinScored(
                    current_f + (max_queue_offset - queue_offset),
                    pos,
                    queue_g,
                ));
                continue;
            }
        }

        // Expand the state.
        let mut double_expanded = match state.status {
            Unvisited => {
                unreachable!("Cannot explore an unvisited node")
            }
            // Expand the currently explored state.
            Explored => {
                state.status = Expanded;
                false
            }
            Expanded => {
                stats.double_expanded += 1;
                true
            }
        };

        // Store the state for copying to matching states.
        let mut state = *state;
        // Matching states will need a match parent.
        state.parent = Parent::match_value();

        // Keep expanding states while we are on a matching diagonal edge.
        // This gives a ~2x speedup on highly similar sequences.
        if loop {
            stats.expanded += 1;
            if DEBUG {
                stats.expanded_states.push(pos);
            }

            // Prune expanded states.
            // TODO: Make this return a new hint?
            // Or just call h manually for a new hint.

            if h.is_start_of_seed(pos) {
                // Check that we don't double expand start-of-seed states.
                // Starts of seeds should only be expanded once.
                assert!(!double_expanded, "Double expanded start of seed {:?}", pos);
                let pq_shift = h.prune_with_hint(pos, hint);
                if REDUCE_RETRIES && pq_shift > 0 {
                    stats.pq_shifts += 1;
                    queue_offset += pq_shift;
                }
            }

            // Retrace path to root and return.
            if pos == target {
                let last = pos;
                let mut path = vec![last];

                let mut current = last;
                // If the state is not in the map, it was found via a match.
                while let Some(previous) = DiagonalMapTrait::get(&states, current)
                    .map_or(Parent::match_value(), |x| x.parent)
                    .parent(&current)
                {
                    path.push(previous);
                    current = previous;
                }

                path.reverse();
                stats.diagonalmap_capacity = states.capacity();
                return Some((g, path, stats));
            }

            if !GREEDY_EDGE_MATCHING_IN_ASTAR {
                break false;
            }

            if let Some(next) = graph.is_match(pos) {
                // Directly expand the next pos, by copying over the current state to there.

                if !DO_NOT_SAVE_GREEDY_MATCHES {
                    let new_state = DiagonalMapTrait::get_mut(&mut states, next);
                    if new_state.g <= state.g {
                        // Continue to the next state in the queue.
                        break true;
                    }
                    double_expanded = if let Expanded = new_state.status {
                        stats.double_expanded += 1;
                        true
                    } else {
                        false
                    };
                    *new_state = state;
                }
                pos = next;

                // NOTE: We do not call h.expand() here, because it isn't needed for pruning-propagation:
                // Pruned positions on this diagonal will always be larger than
                // the expanded positions in front of it, and problems only
                // arise for non-diagonal edges.

                // Count the new state as explored.
                stats.explored += 1;
                stats.skipped_explored += 1;
                if DEBUG {
                    stats.explored_states.push(pos);
                }
            } else {
                break false;
            }
        } {
            continue;
        }

        graph.iterate_outgoing_edges(pos, |next, cost, parent| {
            let next_g = g + cost;

            // Explore next
            let next_state = DiagonalMapTrait::get_mut(&mut states, next);
            if let Unvisited = next_state.status {
                next_state.status = Explored;
            } else if next_g >= next_state.g {
                return;
            };

            let (next_h, next_hint) = h.h_with_hint(next, hint);
            let next_f = next_g + next_h;

            next_state.g = next_g;
            next_state.parent = parent;
            next_state.hint = next_hint;
            queue.push(MinScored(
                next_f + (max_queue_offset - queue_offset),
                next,
                next_g,
            ));

            h.explore(next);
            stats.explored += 1;
            if DEBUG {
                stats.explored_states.push(next);
            }
        });
    }

    None
}
