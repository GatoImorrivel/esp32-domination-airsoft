use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub enum Team {
    Red,
    Blue,
}

#[derive(Debug, Clone, Copy)]
pub struct GameState {
    active: bool,
    current_team: Option<Team>,
    last_tick: Option<Instant>,
    team_red_time: Duration,
    team_blue_time: Duration,
    time_to_win: Duration,
}

impl Default for GameState {
    fn default() -> Self {
        GameState::new(Duration::from_secs(10))
    }
}

impl GameState {
    pub fn new(time_to_win: Duration) -> Self {
        Self {
            active: false,
            current_team: None,
            last_tick: None,
            team_red_time: Duration::ZERO,
            team_blue_time: Duration::ZERO,
            time_to_win,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Start or restart the game
    pub fn start(&mut self) {
        self.active = true;
        self.current_team = None;
        self.last_tick = Some(Instant::now());
        self.team_red_time = Duration::ZERO;
        self.team_blue_time = Duration::ZERO;
        log::info!("Game started");
    }

    /// Stop the game (no more accumulation)
    pub fn stop(&mut self) {
        self.tick();
        self.active = false;
        self.current_team = None;
        self.last_tick = None;
        log::info!("Game stopped");
    }

    /// Handle a button press
    pub fn button_press(&mut self, team: Team) {
        if !self.active {
            return;
        }

        // First, account for time so far
        self.tick();

        // Switch ownership
        self.current_team = Some(team);

        log::info!("{team:#?} pressed the button");
    }

    /// Call this periodically (e.g. every 50–100 ms)
    pub fn tick(&mut self) {
        if !self.active {
            return;
        }

        let now = Instant::now();
        let Some(last) = self.last_tick else {
            self.last_tick = Some(now);
            return;
        };

        let delta = now.duration_since(last);

        if let Some(owner) = self.current_team {
            match owner {
                Team::Blue => self.team_blue_time += delta,
                Team::Red => self.team_red_time += delta,
            }
        }

        self.last_tick = Some(now);
    }

    /// Check if someone won
    pub fn winner(&self) -> Option<Team> {
        if self.team_blue_time >= self.time_to_win {
            Some(Team::Red)
        } else if self.team_red_time >= self.time_to_win {
            Some(Team::Blue)
        } else {
            None
        }
    }

    /// Expose current scores (for UI / WS)
    pub fn scores(&self) -> Scores {
        Scores { red: self.team_red_time, blue: self.team_blue_time }
    }

    /// Who currently owns the point
    pub fn current_team(&self) -> Option<Team> {
        self.current_team
    }
}

pub struct Scores {
    red: Duration,
    blue: Duration
}