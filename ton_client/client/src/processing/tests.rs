use crate::abi::{
    encode_message, encode_message_method, Abi, CallSet, DeploySet, FunctionHeader,
    ParamsOfEncodeMessage, Signer,
};
use crate::error::ApiResult;
use crate::processing::{
    send_message, send_message_method, wait_for_transaction, wait_for_transaction_method,
    CallbackParams, ParamsOfSendMessage, ParamsOfWaitForTransaction, ProcessingEvent,
    ResultOfWaitForTransaction,
};

use crate::processing::types::AbiDecodedOutput;
use crate::tests::{TestClient, EVENTS};

#[tokio::test(core_threads = 2)]
async fn test_wait_message() {
    TestClient::init_log();
    let client = TestClient::new();
    let (events_abi, events_tvc) = TestClient::package(EVENTS, Some(2));
    let keys = client.generate_sign_keys();
    let abi = Abi::Serialized(events_abi.clone());

    let events = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let events_copy = events.clone();
    let callback = move |event: ApiResult<ProcessingEvent>| {
        if let Ok(event) = event {
            events_copy.lock().unwrap().push(event);
        }
    };

    let callback_id = client.register_callback(callback);

    let encode_message = client.wrap_async(encode_message, encode_message_method);
    let send_message = client.wrap_async(send_message, send_message_method);
    let wait_for_transaction = client.wrap_async(wait_for_transaction, wait_for_transaction_method);

    let encoded = encode_message
        .call(ParamsOfEncodeMessage {
            abi: abi.clone(),
            address: None,
            deploy_set: Some(DeploySet {
                workchain_id: None,
                tvc: events_tvc.clone(),
                initial_data: None,
            }),
            call_set: Some(CallSet {
                function_name: "constructor".into(),
                header: Some(FunctionHeader {
                    expire: None,
                    time: None,
                    pubkey: Some(keys.public.clone()),
                }),
                input: None,
            }),
            signer: Signer::WithKeys(keys.clone()),
            processing_try_index: None,
        })
        .await;

    client
        .get_grams_from_giver_async(&encoded.address, None)
        .await;

    let result = send_message
        .call(ParamsOfSendMessage {
            message: encoded.message.clone(),
            callback: Some(CallbackParams::with_id(callback_id)),
        })
        .await;

    let result = wait_for_transaction
        .call(ParamsOfWaitForTransaction {
            message: encoded.message.clone(),
            processing_state: result.processing_state,
            callback: Some(CallbackParams::with_id(callback_id)),
            abi: Some(abi.clone()),
        })
        .await;
    let output = match result {
        ResultOfWaitForTransaction::Complete(output) => Some(output),
        _ => None,
    }.unwrap();

    assert_eq!(output.out_messages.len(), 0);
    assert_eq!(
        output.abi_decoded,
        Some(AbiDecodedOutput {
            out_messages: vec![],
            output: None,
        })
    );
    client.unregister_callback(callback_id);
    let events = events.lock().unwrap().clone();
    let mut events = events.iter();
    assert!(match events.next() {
        Some(ProcessingEvent::WillFetchFirstBlock {}) => true,
        _ => false,
    });
    assert!(match events.next() {
        Some(ProcessingEvent::WillSend { .. }) => true,
        _ => false,
    });
    assert!(match events.next() {
        Some(ProcessingEvent::DidSend { .. }) => true,
        _ => false,
    });
    let mut evt = events.next();
    while match evt {
        Some(ProcessingEvent::WillFetchNextBlock { .. }) => true,
        _ => false,
    } {
        evt = events.next();
    }
    assert!(match evt {
        Some(ProcessingEvent::TransactionReceived { .. }) => true,
        _ => false,
    });
}