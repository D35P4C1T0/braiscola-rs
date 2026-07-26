use briscola_core::card::Card;
use briscola_core::rules::{TrickWinner, trick_winner};
use briscola_core::state::{DeterminizedState, Player};

fn reply_wins(lead_card: Card, reply_card: Card, briscola: briscola_core::card::Suit) -> bool {
    trick_winner(lead_card, reply_card, briscola) == TrickWinner::Follower
}

pub fn choose_lead_card(state: &DeterminizedState, player: Player) -> Card {
    let hand = state.hand(player);

    if let Some(card) = hand
        .iter()
        .copied()
        .filter(|card| card.suit != state.briscola_suit && card.rank.points() == 0)
        .min_by_key(|card| card.rank.power())
    {
        return card;
    }

    if let Some(card) = hand
        .iter()
        .copied()
        .filter(|card| card.suit != state.briscola_suit)
        .min_by_key(|card| (card.rank.points(), card.rank.power()))
    {
        return card;
    }

    hand.iter()
        .copied()
        .min_by_key(|card| (card.rank.points(), card.rank.power()))
        .expect("leader has at least one card")
}

pub fn choose_reply_card(state: &DeterminizedState, player: Player, lead_card: Card) -> Card {
    let hand = state.hand(player);

    let winning_cards: Vec<Card> = hand
        .iter()
        .copied()
        .filter(|card| reply_wins(lead_card, *card, state.briscola_suit))
        .collect();

    if lead_card.rank.points() <= 2 && state.talon.len() > 4 {
        if let Some(card) = winning_cards.iter().copied().min_by_key(|card| {
            (u8::from(card.suit == state.briscola_suit), card.rank.power(), card.rank.points())
        }) {
            return card;
        }
    } else if let Some(card) = winning_cards.iter().copied().min_by_key(|card| {
        (card.rank.points(), card.rank.power(), u8::from(card.suit == state.briscola_suit))
    }) {
        return card;
    }

    hand.iter()
        .copied()
        .min_by_key(|card| {
            (card.rank.points(), u8::from(card.suit == state.briscola_suit), card.rank.power())
        })
        .expect("follower has at least one card")
}

#[cfg(test)]
mod tests {
    use briscola_core::card::{Card, Rank, Suit};
    use briscola_core::state::{DeterminizedState, Player};

    use super::choose_reply_card;

    fn reply_state(opp_hand: Vec<Card>, talon_len: usize) -> DeterminizedState {
        DeterminizedState {
            my_hand: vec![Card::new(Suit::Coins, Rank::Two)],
            opp_hand,
            talon: vec![Card::new(Suit::Cups, Rank::Two); talon_len],
            briscola_suit: Suit::Clubs,
            face_up_trump: Card::new(Suit::Clubs, Rank::King),
            score_me: 0,
            score_opp: 0,
            leader: Player::Me,
            pending_lead: None,
            pending_lead_by: None,
        }
    }

    #[test]
    fn reply_preserves_trump_on_low_value_early_trick() {
        let lead = Card::new(Suit::Swords, Rank::Jack);
        let same_suit_winner = Card::new(Suit::Swords, Rank::Queen);
        let cheap_trump = Card::new(Suit::Clubs, Rank::Two);
        let state = reply_state(vec![same_suit_winner, cheap_trump], 5);

        assert_eq!(choose_reply_card(&state, Player::Opponent, lead), same_suit_winner);
    }

    #[test]
    fn reply_spends_lowest_value_winner_on_late_trick() {
        let lead = Card::new(Suit::Swords, Rank::Jack);
        let same_suit_winner = Card::new(Suit::Swords, Rank::Queen);
        let cheap_trump = Card::new(Suit::Clubs, Rank::Two);
        let state = reply_state(vec![same_suit_winner, cheap_trump], 4);

        assert_eq!(choose_reply_card(&state, Player::Opponent, lead), cheap_trump);
    }
}
