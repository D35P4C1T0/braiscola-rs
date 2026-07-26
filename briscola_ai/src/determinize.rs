use briscola_core::bitset::contains;
use briscola_core::card::full_deck;
use briscola_core::state::{DeterminizedState, Player, PublicGameState};

use crate::rng::FastRng;

/// Errors while building a determinized world from public information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeterminizeError {
    InvalidPublicState,
}

/// Samples a full hidden game state that is consistent with the public view.
pub fn sample_world(
    public: &PublicGameState,
    rng: &mut FastRng,
) -> Result<DeterminizedState, DeterminizeError> {
    if public.last_face_up_trump.suit != public.briscola_suit
        || public.my_hand.len() > briscola_core::card::HAND_SIZE
        || (public.opp_played.is_some() && public.leader != Player::Opponent)
    {
        return Err(DeterminizeError::InvalidPublicState);
    }

    // Explicit public fields are authoritative even if a caller forgot to add
    // them to `seen_cards`.
    let mut cards_in_play = public.my_hand.clone();
    if let Some(card) = public.opp_played {
        cards_in_play.push(card);
    }
    cards_in_play.sort_unstable_by_key(|card| card.index());
    if cards_in_play.windows(2).any(|cards| cards[0] == cards[1]) {
        return Err(DeterminizeError::InvalidPublicState);
    }

    let mut visible_cards = cards_in_play;
    if !visible_cards.contains(&public.last_face_up_trump) {
        visible_cards.push(public.last_face_up_trump);
    }

    let mut unknown = Vec::new();
    for card in full_deck() {
        if !contains(public.seen_cards, card) && !visible_cards.contains(&card) {
            unknown.push(card);
        }
    }

    let expected_opponent_hand_len = expected_opponent_hand_len(public);
    let required_unknown = expected_opponent_hand_len + public.talon_len;
    if unknown.len() < required_unknown {
        return Err(DeterminizeError::InvalidPublicState);
    }

    rng.shuffle(&mut unknown);
    let sampled = &unknown[..required_unknown];
    let opp_hand = sampled[..expected_opponent_hand_len].to_vec();
    let talon = sampled[expected_opponent_hand_len..].to_vec();

    Ok(DeterminizedState {
        my_hand: public.my_hand.clone(),
        opp_hand,
        talon,
        briscola_suit: public.briscola_suit,
        face_up_trump: public.last_face_up_trump,
        score_me: public.score_me,
        score_opp: public.score_opp,
        leader: public.leader,
        pending_lead: public.opp_played,
        pending_lead_by: public.opp_played.map(|_| Player::Opponent),
    })
}

fn expected_opponent_hand_len(public: &PublicGameState) -> usize {
    if public.opp_played.is_some() {
        public.my_hand.len().saturating_sub(1)
    } else {
        public.my_hand.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use briscola_core::bitset::{CardMask, FULL_MASK, add, contains};
    use briscola_core::card::{Card, Rank, Suit, full_deck};
    use briscola_core::state::{Player, PublicGameState};

    fn subtract(full: CardMask, cards: &[Card]) -> CardMask {
        let mut mask = full;
        for card in cards {
            mask &= !(1u64 << card.index());
        }
        mask
    }

    #[test]
    fn sampled_world_uses_only_unknown_cards() {
        let my_cards = vec![
            Card::new(Suit::Coins, Rank::Ace),
            Card::new(Suit::Cups, Rank::Three),
            Card::new(Suit::Swords, Rank::King),
        ];
        let opp_played = Card::new(Suit::Clubs, Rank::Two);
        let face_up = Card::new(Suit::Clubs, Rank::Ace);

        let mut fixed_seen = my_cards.clone();
        fixed_seen.push(opp_played);
        fixed_seen.push(face_up);

        let mut unknown_target = Vec::new();
        for card in full_deck() {
            if !fixed_seen.contains(&card) && unknown_target.len() < 12 {
                unknown_target.push(card);
            }
        }

        let seen = subtract(FULL_MASK, &unknown_target);

        let public = PublicGameState {
            my_hand: my_cards,
            opp_played: Some(opp_played),
            briscola_suit: Suit::Clubs,
            talon_len: 10,
            last_face_up_trump: face_up,
            seen_cards: seen,
            score_me: 20,
            score_opp: 11,
            leader: Player::Opponent,
        };

        let mut rng = FastRng::new(7);
        let sampled = sample_world(&public, &mut rng).expect("sample world");

        assert_eq!(sampled.opp_hand.len(), 2);
        assert_eq!(sampled.talon.len(), 10);

        for card in sampled.opp_hand.iter().chain(sampled.talon.iter()) {
            assert!(!contains(public.seen_cards, *card));
        }

        let mut union = 0u64;
        for card in sampled.opp_hand.iter().chain(sampled.talon.iter()) {
            union = add(union, *card);
        }

        for card in unknown_target {
            assert!(contains(union, card));
        }
    }

    #[test]
    fn explicit_visible_cards_are_never_sampled_when_seen_mask_is_incomplete() {
        let my_cards = vec![Card::new(Suit::Coins, Rank::Ace), Card::new(Suit::Cups, Rank::Three)];
        let opp_played = Card::new(Suit::Swords, Rank::King);
        let face_up = Card::new(Suit::Clubs, Rank::Four);
        let public = PublicGameState {
            my_hand: my_cards.clone(),
            opp_played: Some(opp_played),
            briscola_suit: Suit::Clubs,
            talon_len: 10,
            last_face_up_trump: face_up,
            seen_cards: 0,
            score_me: 20,
            score_opp: 11,
            leader: Player::Opponent,
        };

        let mut rng = FastRng::new(17);
        let sampled = sample_world(&public, &mut rng).expect("sample world");
        let hidden_cards = sampled.opp_hand.iter().chain(sampled.talon.iter());

        for card in my_cards.into_iter().chain([opp_played, face_up]) {
            assert!(!hidden_cards.clone().any(|hidden| *hidden == card));
        }
    }

    #[test]
    fn duplicate_explicit_cards_are_rejected() {
        let duplicate = Card::new(Suit::Coins, Rank::Ace);
        let public = PublicGameState {
            my_hand: vec![duplicate],
            opp_played: Some(duplicate),
            briscola_suit: Suit::Clubs,
            talon_len: 0,
            last_face_up_trump: Card::new(Suit::Clubs, Rank::Four),
            seen_cards: 0,
            score_me: 0,
            score_opp: 0,
            leader: Player::Opponent,
        };

        let mut rng = FastRng::new(17);
        assert!(matches!(
            sample_world(&public, &mut rng),
            Err(DeterminizeError::InvalidPublicState)
        ));
    }

    #[test]
    fn drawn_face_up_trump_can_be_in_hand_after_talon_is_empty() {
        let face_up = Card::new(Suit::Clubs, Rank::Four);
        let public = PublicGameState {
            my_hand: vec![face_up],
            opp_played: None,
            briscola_suit: Suit::Clubs,
            talon_len: 0,
            last_face_up_trump: face_up,
            seen_cards: add(0, face_up),
            score_me: 50,
            score_opp: 50,
            leader: Player::Me,
        };

        let mut rng = FastRng::new(17);
        let sampled = sample_world(&public, &mut rng).expect("sample endgame world");

        assert_eq!(sampled.my_hand, vec![face_up]);
        assert_eq!(sampled.opp_hand.len(), 1);
        assert!(sampled.talon.is_empty());
    }
}
