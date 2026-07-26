# brAIscola-rs

A Rust implementation of a 2-player Briscola engine with hidden-information Monte Carlo move selection.

## Workspace Layout

- `briscola_core`: card model, scoring/power ordering, trick resolution, deterministic state transitions.
- `briscola_ai`: determinization from public information, heuristic rollout policy, root Monte Carlo evaluator.
- `cli`: binaries for move evaluation, full game simulation, and move-advisor workflows (`interactive` + `suggest`).

## Implemented Rules

- 40-card Italian deck (`Coins`, `Cups`, `Swords`, `Clubs`) with ranks `A,2,3,4,5,6,7,J,Q,K`.
- Briscola scoring values: `A=11`, `3=10`, `K=4`, `Q=3`, `J=2`, others `0`.
- Trick strength ordering: `A > 3 > K > Q > J > 7 > 6 > 5 > 4 > 2`.
- Winner draws first; loser draws second.
- When one talon card remains, loser receives the face-up trump card as final draw.
- No obligation to follow suit; legal move set is always all cards in hand.

## AI Approach

At each decision:

1. Build unknown pool from `FULL_DECK - seen_cards`.
2. Sample hidden world consistent with:
   - opponent hand size,
   - talon length,
   - known face-up trump.
3. Force each candidate move at root.
4. Roll out to terminal state with Briscola heuristics for both players.
5. Aggregate `p_win` and expected score delta; choose argmax.

## Run

```bash
cargo test
cargo run -p cli --bin cli
cargo run -p cli --bin simulate_game -- 42
cargo run -p cli --bin simulate_game -- 42 --best-me --samples 256
cargo run -p cli --bin advisor -- interactive --samples 128 --seed 42
cargo run -p cli --bin advisor -- suggest --json /path/to/turn_n.json --samples 128 --seed 42
cargo run -p cli --bin play_tui --
cargo run -p cli --bin play_tui -- 42 --hint-samples 128 --opponent-samples 96
```

## More Examples

Direct commands:

```bash
# 1) Evaluate best move from a built-in state
cargo run -p cli --bin cli

# 2) Simulate a full game (heuristic policy)
cargo run -p cli --bin simulate_game -- 99

# 3) Simulate a full game with best-move policy for Me
cargo run -p cli --bin simulate_game -- 99 --best-me --samples 256

# 4) Suggest move when opponent already played in current trick
cargo run -p cli --bin advisor -- suggest \
  --json examples/advisor/respond_turn_n.json \
  --samples 128 --seed 13

# 5) Suggest move when it is your turn to lead
cargo run -p cli --bin advisor -- suggest \
  --json examples/advisor/lead_turn_n.json \
  --samples 192 --seed 42

# 6) Endgame suggestion example
cargo run -p cli --bin advisor -- suggest \
  --json examples/advisor/endgame_turn_n.json \
  --samples 256 --seed 7
```

Script wrappers:

```bash
# run single examples
bash scripts/example_move_eval.sh
bash scripts/example_simulate.sh 42
bash scripts/example_simulate_best_me.sh 42 128
bash scripts/example_advisor_suggest.sh examples/advisor/respond_turn_n.json 128 13
bash scripts/example_advisor_interactive.sh 128 42
bash scripts/example_play_tui.sh 42 128 96

# run all non-interactive examples
bash scripts/run_all_examples.sh
```

Example files:
- `examples/advisor/respond_turn_n.json`
- `examples/advisor/lead_turn_n.json`
- `examples/advisor/endgame_turn_n.json`

- `cargo run -p cli --bin cli` prints the selected best move and statistics for every legal move.
- `cargo run -p cli --bin simulate_game -- <seed>` simulates an entire game and prints every trick.
- Add `--best-me` to have `Me` choose root Monte Carlo best moves each turn.
- Tune `--samples <N>` to trade speed for stronger move selection in `--best-me` mode.
- `cargo run -p cli --bin advisor -- interactive` starts a persistent session that tracks game state turn by turn.
- `cargo run -p cli --bin advisor -- suggest --json ...` computes the best move for turn `N+1` from a JSON description of turns `1..N`.
- Advisor modes render terminal ASCII card art from `res/napoletane/*.webp`; white card background is treated as transparent.
- `cargo run -p cli --bin play_tui -- ...` launches a fully interactive game where you play as `Me` against the AI.
- `play_tui` defaults to a random seed if no seed is provided (`play_tui -- 42` or `--seed 42` for reproducible runs).
- For the full card-table layout, use a terminal window of at least `82x36`; smaller
  windows show a resize prompt instead of clipping the card art.

### TUI Controls

- `Left/Right` or `1..3`: select card in your hand.
- `Enter` or `Space`: play selected card.
- `h`: toggle best-move hint on/off.
- `q` or `Esc`: quit.

The TUI automatically handles:
- deck setup and talon draws,
- opponent turns,
- score updates,
- trick winner calculation,
- card counts and trump display on screen,
- a dedicated table-cards panel that shows current table cards,
- end-of-turn winner feedback: the winning table card flashes with a green border.

## Advisor JSON Format

`advisor suggest` expects a JSON object with this shape:

```json
{
  "briscola_suit": "clubs",
  "face_up_trump": "🪄4",
  "my_hand": ["🪙A", "⚔️2"],
  "opp_played": "🪙K",
  "talon_len": 0,
  "score_me": 50,
  "score_opp": 48,
  "leader": "opponent",
  "history": [
    { "lead": "⚔️4", "reply": "⚔️7" }
  ],
  "seen_cards": ["🪄K"],
  "samples_per_move": 128,
  "seed": 42
}
```

Required fields:
- `briscola_suit`: briscola suit (`coins|cups|swords|clubs`, aliases `o|u|s|c`, `d|b`, or suit emojis).
- `face_up_trump`: face-up trump card.
- `my_hand`: your current hand (1 to 3 cards).
- `talon_len`: cards left in talon.
- `score_me`: your current score.
- `score_opp`: opponent score.
- `leader`: current trick leader (`me` or `opponent`, also accepts `m/player` and `opp/o`).

Optional fields:
- `opp_played`: opponent lead card if opponent has already played in current trick.
- `history`: completed tricks from turns `1..N`, each with `{ "lead": "...", "reply": "..." }`.
- `seen_cards`: extra known cards to include in visible information.
- `samples_per_move`: Monte Carlo samples per legal move (default `128`).
- `seed`: RNG seed for reproducibility (default `42`).

Card format accepted:
- Compact form: `<suit><rank>` like `🪙A`, `🪄K`, `⚔️3`, `🏆7`.
- Legacy compact aliases are still accepted: `oA`, `cK`, `s3`, `u7` (plus `d` for denari and `b` for bastoni).
- Explicit form: `<suit>:<rank>` like `clubs:K`, `coins:A`.
- Suit emoji mapping: `🏆` coppe, `🪙` denari, `⚔️` spade, `🪄` bastoni.
- Rank tokens: `A,2,3,4,5,6,7,J,Q,K` (also accepts names like `ace`, `king`, `fante`, `re`).

Advisor mode behavior:
- `interactive` keeps a live session until game end and suggests best moves each trick.
- `suggest` is stateless and evaluates only the next decision (`N+1`) from the JSON snapshot.

## Quality Gates

The workspace enforces strict clippy/rust lints via `Cargo.toml` workspace lint settings and uses Rust 2024 style formatting.

```bash
cargo fmt-all
cargo fmt-check
cargo lint-strict
cargo verify
```
