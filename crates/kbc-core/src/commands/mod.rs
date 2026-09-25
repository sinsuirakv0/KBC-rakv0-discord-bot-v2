//! 組み込みCommandを登録する。

pub(crate) mod common;
mod eventdata;
mod gatya;
mod help;
mod item;
mod maint;
mod push;
mod sale;
pub(crate) mod skd;
mod st;
mod static_response;
mod tut;
mod ut;

use std::path::PathBuf;
use std::sync::Arc;

use crate::command::Command;
use crate::content::ContentCatalog;
use crate::services::{Clock, HttpService};
use crate::storage::StorageService;
use crate::task_runtime::TaskRuntime;
use eventdata::EventDataCommand;
use gatya::{GatyaCommand, RegisteredGatyaDataSource};
use help::HelpCommand;
use item::{ItemCommand, RegisteredItemDataSource};
use maint::MaintCommand;
use push::PushCommand;
use sale::{RegisteredSaleDataSource, SaleCommand};
use skd::SkdCommand;
use st::StCommand;
use static_response::StaticResponseCommand;
use tut::TutCommand;
use ut::UtCommand;

pub(crate) fn built_in_commands(
    content: &ContentCatalog,
    http: Arc<HttpService>,
    clock: Arc<dyn Clock>,
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
    ffmpeg_path: Option<PathBuf>,
) -> Result<Vec<Box<dyn Command>>, MissingCommandContent> {
    let mut commands: Vec<Box<dyn Command>> = content
        .responses()
        .map(|(name, response)| {
            let help = content.help(name).unwrap_or(response);
            Box::new(StaticResponseCommand::new(name, response, help)) as Box<dyn Command>
        })
        .collect();
    let help_index = content
        .help("index")
        .ok_or(MissingCommandContent::HelpIndex)?;
    commands.push(Box::new(HelpCommand::new(help_index)));
    let push_help = content
        .help("push")
        .ok_or(MissingCommandContent::CommandHelp("push"))?;
    commands.push(Box::new(PushCommand::new(push_help, Arc::clone(&storage))));
    let maint_help = content
        .help("maint")
        .ok_or(MissingCommandContent::CommandHelp("maint"))?;
    commands.push(Box::new(MaintCommand::new(
        maint_help,
        Arc::clone(&storage),
        Arc::clone(&tasks),
    )));
    let item_help = content
        .help("item")
        .ok_or(MissingCommandContent::CommandHelp("item"))?;
    commands.push(Box::new(ItemCommand::new(
        item_help,
        RegisteredItemDataSource::new(Arc::clone(&http)),
        Arc::clone(&clock),
    )));
    let sale_help = content
        .help("sale")
        .ok_or(MissingCommandContent::CommandHelp("sale"))?;
    commands.push(Box::new(SaleCommand::new(
        sale_help,
        RegisteredSaleDataSource::new(Arc::clone(&http)),
        Arc::clone(&clock),
    )));
    let st_help = content
        .help("st")
        .ok_or(MissingCommandContent::CommandHelp("st"))?;
    commands.push(Box::new(StCommand::new(st_help, Arc::clone(&http))));
    let skd_help = content
        .help("skd")
        .ok_or(MissingCommandContent::CommandHelp("skd"))?;
    commands.push(Box::new(SkdCommand::new(
        skd_help,
        Arc::clone(&http),
        Arc::clone(&storage),
    )));
    let tut_help = content
        .help("tut")
        .ok_or(MissingCommandContent::CommandHelp("tut"))?;
    commands.push(Box::new(TutCommand::new(
        tut_help,
        Arc::clone(&http),
        Arc::clone(&tasks),
        ffmpeg_path.clone(),
    )));
    let ut_help = content
        .help("ut")
        .ok_or(MissingCommandContent::CommandHelp("ut"))?;
    commands.push(Box::new(UtCommand::new(
        ut_help,
        Arc::clone(&http),
        tasks,
        ffmpeg_path,
    )));
    let gatya_help = content
        .help("gatya")
        .ok_or(MissingCommandContent::CommandHelp("gatya"))?;
    commands.push(Box::new(GatyaCommand::new(
        gatya_help,
        RegisteredGatyaDataSource::new(Arc::clone(&http)),
        Arc::clone(&clock),
    )));
    let event_data_help = content
        .help("eventdata")
        .ok_or(MissingCommandContent::CommandHelp("eventdata"))?;
    commands.push(Box::new(EventDataCommand::new(
        event_data_help,
        Arc::clone(&http),
        clock,
    )));

    Ok(commands)
}

#[derive(Debug)]
pub(crate) enum MissingCommandContent {
    HelpIndex,
    CommandHelp(&'static str),
}

impl std::fmt::Display for MissingCommandContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HelpIndex => formatter.write_str("missing command help index"),
            Self::CommandHelp(name) => write!(formatter, "missing help content for {name}"),
        }
    }
}
