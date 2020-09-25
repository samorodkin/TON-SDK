/*
* Copyright 2018-2020 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and
* limitations under the License.
*/

use crate::dispatch::DispatchTable;

#[cfg(test)]
mod tests;

mod errors;
mod process_message;
mod types;
mod internal;
mod wait_for_transaction;
mod send_message;
mod defaults;

pub use errors::{Error, ErrorCode};
pub use process_message::{
    process_message, CallbackParams, MessageMonitoringOptions, MessageProcessingEvent,
    MessageSource, ParamsOfProcessMessage, ResultOfProcessMessage,
};

pub(crate) fn register(handlers: &mut DispatchTable) {
    handlers.register_api_types("net", vec![
        CallbackParams::type_info,
    ]);
    handlers.spawn("net.process_message", process_message);
}
