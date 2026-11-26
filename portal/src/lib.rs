//! Player accounts portal for the MAG replacement server.
//!
//! Players sign in with Google, claim a player name and link their launcher. The launcher
//! trades its device token for short-lived signed game tickets, which the game server
//! verifies with the portal's public key (`docs/portal-ticket.md`).

pub mod admin;
pub mod admin_web;
pub mod config;
pub mod db;
pub mod game_admin;
pub mod names;
pub mod oidc;
pub mod ratelimit;
pub mod report_store;
pub mod reports;
pub mod social;
pub mod social_web;
pub mod staff;
pub mod status;
pub mod ticket;
pub mod tokens;
pub mod web;
