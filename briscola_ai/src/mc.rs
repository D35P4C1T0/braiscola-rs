use std::cmp::Ordering;

use briscola_core::card::Card;
use briscola_core::state::{Player, PublicGameState, StateError};

use crate::determinize::{DeterminizeError, sample_world};
use crate::rng::FastRng;
use crate::rollout::{choose_lead_card, choose_reply_card};

/// Monte Carlo search configuration.
#[derive(Debug, Clone, Copy)]
pub struct MonteCarloConfig {
    pub samples_per_move: usize,
}

/// Aggregated statistics for a legal move.
#[derive(Debug, Clone, Copy)]
pub struct MoveStats {
    pub card: Card,
    pub p_win: f64,
    pub expected_score_delta: f64,
    pub n_samples: usize,
}

/// Ranked result for all legal moves.
#[derive(Debug, Clone)]
pub struct BestMoveResult {
    pub best_move: Card,
    pub moves: Vec<MoveStats>,
}

/// Failures that can occur while selecting a move by simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonteCarloError {
    NoLegalMoves,
    NotMyTurn,
    DeterminizeFailed,
    SimulationFailed,
    NoSuccessfulSamples,
}

/// Chooses the best move among legal options using sampled hidden information.
pub fn choose_best_move(
    public: &PublicGameState,
    rng: &mut FastRng,
    config: MonteCarloConfig,
) -> Result<BestMoveResult, MonteCarloError> {
    let legal_moves = public.legal_moves();
    if legal_moves.is_empty() {
        return Err(MonteCarloError::NoLegalMoves);
    }

    if public.opp_played.is_none() && public.leader != Player::Me {
        return Err(MonteCarloError::NotMyTurn);
    }

    let samples = config.samples_per_move.max(1);
    let mut totals = vec![(0.0_f64, 0.0_f64, 0_usize); legal_moves.len()];

    // Evaluate every candidate against the same sampled worlds. Besides requiring
    // fewer determinizations, this reduces variance when comparing close moves.
    for _ in 0..samples {
        let sampled_state = sample_world(public, rng)
            .map_err(|_: DeterminizeError| MonteCarloError::DeterminizeFailed)?;

        for (index, card) in legal_moves.iter().copied().enumerate() {
            let mut state = sampled_state.clone();
            let (win_score, delta) = simulate_with_forced_move(public, &mut state, card)?;
            totals[index].0 += win_score;
            totals[index].1 += f64::from(delta);
            totals[index].2 += 1;
        }
    }

    let mut moves = Vec::with_capacity(legal_moves.len());
    for (card, (wins, deltas, successful_samples)) in
        legal_moves.into_iter().zip(totals.into_iter())
    {
        if successful_samples == 0 {
            return Err(MonteCarloError::NoSuccessfulSamples);
        }

        let successful_u32 =
            u32::try_from(successful_samples).map_err(|_| MonteCarloError::SimulationFailed)?;
        let n = f64::from(successful_u32);

        moves.push(MoveStats {
            card,
            p_win: wins / n,
            expected_score_delta: deltas / n,
            n_samples: successful_samples,
        });
    }

    moves.sort_by(|a, b| {
        b.p_win.partial_cmp(&a.p_win).unwrap_or(std::cmp::Ordering::Equal).then_with(|| {
            b.expected_score_delta
                .partial_cmp(&a.expected_score_delta)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    Ok(BestMoveResult { best_move: moves[0].card, moves })
}

fn simulate_with_forced_move(
    public: &PublicGameState,
    state: &mut briscola_core::state::DeterminizedState,
    forced_move: Card,
) -> Result<(f64, i16), MonteCarloError> {
    if public.opp_played.is_some() {
        state
            .play_reply_card(Player::Me, forced_move)
            .map_err(|_: StateError| MonteCarloError::SimulationFailed)?;
    } else {
        state
            .play_lead_card(Player::Me, forced_move)
            .map_err(|_: StateError| MonteCarloError::SimulationFailed)?;
        let opp_reply = choose_reply_card(state, Player::Opponent, forced_move);
        state
            .play_reply_card(Player::Opponent, opp_reply)
            .map_err(|_: StateError| MonteCarloError::SimulationFailed)?;
    }

    while !state.is_terminal() {
        if let Some(lead_card) = state.pending_lead {
            let lead_by = state.pending_lead_by.ok_or(MonteCarloError::SimulationFailed)?;
            let follower = lead_by.other();
            let reply = choose_reply_card(state, follower, lead_card);
            state
                .play_reply_card(follower, reply)
                .map_err(|_: StateError| MonteCarloError::SimulationFailed)?;
        } else {
            let leader = state.leader;
            let lead = choose_lead_card(state, leader);
            state
                .play_lead_card(leader, lead)
                .map_err(|_: StateError| MonteCarloError::SimulationFailed)?;
            let follower = leader.other();
            let reply = choose_reply_card(state, follower, lead);
            state
                .play_reply_card(follower, reply)
                .map_err(|_: StateError| MonteCarloError::SimulationFailed)?;
        }
    }

    let delta = i16::from(state.score_me) - i16::from(state.score_opp);
    let win_score = match delta.cmp(&0) {
        Ordering::Greater => 1.0,
        Ordering::Equal => 0.5,
        Ordering::Less => 0.0,
    };

    Ok((win_score, delta))
}

#[cfg(test)]
mod tests {
    use briscola_core::bitset::{FULL_MASK, card_mask};
    use briscola_core::card::{Card, Rank, Suit};
    use briscola_core::state::{Player, PublicGameState};

    use super::*;

    #[test]
    fn follower_prefers_winning_card_in_deterministic_end_state() {
        let winning = Card::new(Suit::Coins, Rank::Ace);
        let losing = Card::new(Suit::Swords, Rank::Two);
        let hidden = Card::new(Suit::Clubs, Rank::King);
        let opp_played = Card::new(Suit::Coins, Rank::King);
        let trump = Card::new(Suit::Clubs, Rank::Four);

        let public = PublicGameState {
            my_hand: vec![winning, losing],
            opp_played: Some(opp_played),
            briscola_suit: Suit::Clubs,
            talon_len: 0,
            last_face_up_trump: trump,
            seen_cards: FULL_MASK & !card_mask(hidden),
            score_me: 50,
            score_opp: 48,
            leader: Player::Opponent,
        };

        let mut rng = FastRng::new(13);
        let result = choose_best_move(&public, &mut rng, MonteCarloConfig { samples_per_move: 8 })
            .expect("choose move");

        assert_eq!(result.best_move, winning);
        assert!(result.moves.iter().all(|stats| stats.n_samples == 8));
    }
}
