use std::{collections::HashMap, net::SocketAddr, time::SystemTime};

use color_eyre::Result;
use context_attribute::context;
use framework::{AdditionalOutput, MainOutput, PerceptionInput};
use hardware::{SpeakerInterface, TimeInterface};
use serde::{Deserialize, Serialize};
use types::{
    audio::{Sound, SpeakerRequest},
    cycle_time::CycleTime,
    game_controller_state::GameControllerState,
    messages::IncomingMessage,
};

#[derive(Deserialize, Serialize)]
pub struct GameControllerFilter {
    game_controller_state: Option<GameControllerState>,
    last_game_state_change: Option<SystemTime>,

    last_contact: HashMap<SocketAddr, SystemTime>,
    last_collision_warning: Option<SystemTime>,
}

#[context]
pub struct CreationContext {}

#[context]
pub struct CycleContext {
    hardware_interface: HardwareInterface,
    cycle_time: Input<CycleTime, "cycle_time">,
    network_message: PerceptionInput<Option<IncomingMessage>, "SplNetwork", "filtered_message?">,

    last_contact:
        AdditionalOutput<HashMap<SocketAddr, SystemTime>, "game_controller_address_contacts_times">,
}

#[context]
#[derive(Default)]
pub struct MainOutputs {
    pub game_controller_state: MainOutput<Option<GameControllerState>>,
    pub game_controller_address: MainOutput<Option<SocketAddr>>,
}

impl GameControllerFilter {
    pub fn new(_context: CreationContext) -> Result<Self> {
        Ok(Self {
            game_controller_state: None,
            last_game_state_change: None,
            last_contact: HashMap::new(),
            last_collision_warning: None,
        })
    }

    pub fn cycle(
        &mut self,
        mut context: CycleContext<impl TimeInterface + SpeakerInterface>,
    ) -> Result<MainOutputs> {
        for (time, source_address, game_controller_state_message) in context
            .network_message
            .persistent
            .iter()
            .flat_map(|(time, messages)| messages.iter().flatten().map(|message| (*time, *message)))
            .filter_map(|(time, message)| match message {
                IncomingMessage::GameController(source_address, message) => {
                    Some((time, source_address, message))
                }
                _ => None,
            })
        {
            let game_state_changed = match &self.game_controller_state {
                Some(game_controller_state) => {
                    game_controller_state.game_state != game_controller_state_message.game_state
                }
                None => true,
            };
            if game_state_changed {
                self.last_game_state_change = Some(context.cycle_time.start_time);
            }
            self.game_controller_state = Some(GameControllerState {
                game_state: game_controller_state_message.game_state,
                game_phase: game_controller_state_message.game_phase,
                kicking_team: game_controller_state_message.kicking_team,
                last_game_state_change: self.last_game_state_change.unwrap(),
                penalties: game_controller_state_message.hulks_team.clone().into(),
                remaining_amount_of_messages: game_controller_state_message
                    .hulks_team
                    .remaining_amount_of_messages,
                sub_state: game_controller_state_message.sub_state,
                hulks_team_is_home_after_coin_toss: game_controller_state_message
                    .hulks_team_is_home_after_coin_toss,
            });

            self.last_contact.insert(*source_address, time);
            let on_cooldown = self
                .last_collision_warning
                .is_some_and(|last_collision_warning| {
                    time.duration_since(last_collision_warning)
                        .expect("time ran backwards")
                        .as_secs_f32()
                        < 10.0
                });
            let recent_contacts = self.last_contact.iter().filter(|(_address, last_contact)| {
                time.duration_since(**last_contact)
                    .expect("time ran backwards")
                    .as_secs_f32()
                    < 5.0
            });
            let collisions = recent_contacts.count() > 1;

            if collisions && !on_cooldown {
                context
                    .hardware_interface
                    .write_to_speakers(SpeakerRequest::PlaySound {
                        sound: Sound::GameControllerCollision,
                    });
                self.last_collision_warning = Some(time);
            }
        }

        context
            .last_contact
            .fill_if_subscribed(|| self.last_contact.clone());

        let last_address = self
            .last_contact
            .iter()
            .max_by_key(|(_address, time)| *time)
            .map(|(address, _time)| *address);

        Ok(MainOutputs {
            game_controller_state: self.game_controller_state.into(),
            game_controller_address: last_address.into(),
        })
    }
}
