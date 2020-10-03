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

use super::JsonResponse;
use crate::client::{Callback, ClientContext};
use crate::error::{ApiError, ApiResult};
use api_doc::reflect::TypeInfo;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::marker::PhantomData;

use api_doc::api::{Field, Module, API};
#[cfg(feature = "node_interaction")]
use std::future::Future;

impl JsonResponse {
    pub(crate) fn from_result(result_json: String) -> Self {
        Self {
            result_json,
            error_json: String::new(),
        }
    }

    pub(crate) fn from_error(err: ApiError) -> Self {
        JsonResponse {
            result_json: String::new(),
            error_json: serde_json::to_string(&err)
                .unwrap_or(r#"{"category": "sdk", "code": 1, "message": ""}"#.to_string()),
        }
    }
}

trait SyncHandler {
    fn handle(&self, context: std::sync::Arc<ClientContext>, params_json: &str) -> JsonResponse;
}

trait AsyncHandler {
    fn handle(
        &self,
        context: std::sync::Arc<ClientContext>,
        params_json: String,
        request_id: u32,
        on_result: Box<Callback>,
    );
}

pub(crate) struct DispatchTable {
    pub(crate) api: API,

    sync_runners: HashMap<String, Box<dyn SyncHandler + Sync>>,
    async_runners: HashMap<String, Box<dyn AsyncHandler + Sync>>,
}

pub(crate) fn parse_params<P: DeserializeOwned>(params_json: &str) -> ApiResult<P> {
    serde_json::from_str(params_json).map_err(|err| ApiError::invalid_params(params_json, err))
}

struct RawAsyncHandler<F>
where
    F: Fn(std::sync::Arc<ClientContext>, String, u32, Box<Callback>),
{
    handler: F,
}

impl<F> RawAsyncHandler<F>
where
    F: Fn(std::sync::Arc<ClientContext>, String, u32, Box<Callback>),
{
    pub fn new(handler: F) -> Self {
        Self { handler }
    }
}

impl<F> AsyncHandler for RawAsyncHandler<F>
where
    F: Fn(std::sync::Arc<ClientContext>, String, u32, Box<Callback>),
{
    fn handle(
        &self,
        context: std::sync::Arc<ClientContext>,
        params_json: String,
        request_id: u32,
        on_result: Box<Callback>,
    ) {
        (self.handler)(context, params_json, request_id, on_result)
    }
}

#[cfg(feature = "node_interaction")]
struct SpawnHandler<P, R, Fut, F>
where
    P: Send + DeserializeOwned + 'static,
    R: Send + Serialize + 'static,
    Fut: Future<Output = ApiResult<R>> + 'static,
    F: Send + Fn(std::sync::Arc<ClientContext>, P) -> Fut + 'static,
{
    handler: std::sync::Arc<F>,
    // Mutex is needed to have Sync trait implemented for struct
    phantom: PhantomData<std::sync::Mutex<(P, R, Fut)>>,
}

#[cfg(feature = "node_interaction")]
impl<P, R, Fut, F> SpawnHandler<P, R, Fut, F>
where
    P: Send + DeserializeOwned + 'static,
    R: Send + Serialize + 'static,
    Fut: Future<Output = ApiResult<R>> + 'static,
    F: Send + Fn(std::sync::Arc<ClientContext>, P) -> Fut + 'static,
{
    pub fn new(handler: F) -> Self {
        Self {
            handler: std::sync::Arc::new(handler),
            phantom: PhantomData,
        }
    }
}

#[cfg(feature = "node_interaction")]
impl<P, R, Fut, F> AsyncHandler for SpawnHandler<P, R, Fut, F>
where
    P: Send + DeserializeOwned + 'static,
    R: Send + Serialize + 'static,
    Fut: Send + Future<Output = ApiResult<R>> + 'static,
    F: Send + Sync + Fn(std::sync::Arc<ClientContext>, P) -> Fut + 'static,
{
    fn handle(
        &self,
        context: std::sync::Arc<ClientContext>,
        params_json: String,
        request_id: u32,
        on_result: Box<Callback>,
    ) {
        let handler = self.handler.clone();
        let context_copy = context.clone();
        context.async_runtime_handle.enter(move || {
            tokio::spawn(async move {
                let result = match parse_params(&params_json) {
                    Ok(params) => {
                        let result = handler(context_copy, params).await;
                        match result {
                            Ok(result) => {
                                JsonResponse::from_result(serde_json::to_string(&result).unwrap())
                            }
                            Err(err) => JsonResponse::from_error(err),
                        }
                    }
                    Err(err) => JsonResponse::from_error(err),
                };
                result.send(&*on_result, request_id, 1);
            });
        });
    }
}

#[cfg(feature = "node_interaction")]
struct SpawnNoArgsHandler<R, Fut, F>
where
    R: Send + Serialize + 'static,
    Fut: Future<Output = ApiResult<R>> + 'static,
    F: Send + Fn(std::sync::Arc<ClientContext>) -> Fut + 'static,
{
    handler: std::sync::Arc<F>,
    // Mutex is needed to have Sync trait implemented for struct
    phantom: PhantomData<std::sync::Mutex<(R, Fut)>>,
}

#[cfg(feature = "node_interaction")]
impl<R, Fut, F> SpawnNoArgsHandler<R, Fut, F>
where
    R: Send + Serialize + 'static,
    Fut: Future<Output = ApiResult<R>> + 'static,
    F: Send + Fn(std::sync::Arc<ClientContext>) -> Fut + 'static,
{
    pub fn new(handler: F) -> Self {
        Self {
            handler: std::sync::Arc::new(handler),
            phantom: PhantomData,
        }
    }
}

#[cfg(feature = "node_interaction")]
impl<R, Fut, F> AsyncHandler for SpawnNoArgsHandler<R, Fut, F>
where
    R: Send + Serialize + 'static,
    Fut: Send + Future<Output = ApiResult<R>> + 'static,
    F: Send + Sync + Fn(std::sync::Arc<ClientContext>) -> Fut + 'static,
{
    fn handle(
        &self,
        context: std::sync::Arc<ClientContext>,
        _params_json: String,
        request_id: u32,
        on_result: Box<Callback>,
    ) {
        let handler = self.handler.clone();
        let context_copy = context.clone();
        context.async_runtime_handle.enter(move || {
            tokio::spawn(async move {
                let result = handler(context_copy).await;
                let result = match result {
                    Ok(result) => {
                        JsonResponse::from_result(serde_json::to_string(&result).unwrap())
                    }
                    Err(err) => JsonResponse::from_error(err),
                };
                result.send(&*on_result, request_id, 1);
            });
        });
    }
}

struct CallHandler<P, R, F>
where
    P: Send + DeserializeOwned,
    R: Send + Serialize,
    F: Fn(std::sync::Arc<ClientContext>, P) -> ApiResult<R>,
{
    handler: F,
    phantom: PhantomData<std::sync::Mutex<(P, R)>>,
}

impl<P, R, F> CallHandler<P, R, F>
where
    P: Send + DeserializeOwned,
    R: Send + Serialize,
    F: Fn(std::sync::Arc<ClientContext>, P) -> ApiResult<R>,
{
    pub fn new(handler: F) -> Self {
        Self {
            handler,
            phantom: PhantomData,
        }
    }
}

impl<P, R, F> SyncHandler for CallHandler<P, R, F>
where
    P: Send + DeserializeOwned,
    R: Send + Serialize,
    F: Fn(std::sync::Arc<ClientContext>, P) -> ApiResult<R>,
{
    fn handle(&self, context: std::sync::Arc<ClientContext>, params_json: &str) -> JsonResponse {
        match parse_params(params_json) {
            Ok(params) => {
                let result = (self.handler)(context, params);
                match result {
                    Ok(result) => {
                        JsonResponse::from_result(serde_json::to_string(&result).unwrap())
                    }
                    Err(err) => JsonResponse::from_error(err),
                }
            }
            Err(err) => JsonResponse::from_error(err),
        }
    }
}

struct CallNoArgsHandler<R, F>
where
    R: Send + Serialize,
    F: Fn(std::sync::Arc<ClientContext>) -> ApiResult<R>,
{
    handler: F,
    phantom: PhantomData<std::sync::Mutex<R>>,
}

impl<R, F> CallNoArgsHandler<R, F>
where
    R: Send + Serialize,
    F: Fn(std::sync::Arc<ClientContext>) -> ApiResult<R>,
{
    pub fn new(handler: F) -> Self {
        Self {
            handler,
            phantom: PhantomData,
        }
    }
}

impl<R, F> SyncHandler for CallNoArgsHandler<R, F>
where
    R: Send + Serialize,
    F: Fn(std::sync::Arc<ClientContext>) -> ApiResult<R>,
{
    fn handle(&self, context: std::sync::Arc<ClientContext>, _params_json: &str) -> JsonResponse {
        let result = (self.handler)(context);
        match result {
            Ok(result) => JsonResponse::from_result(serde_json::to_string(&result).unwrap()),
            Err(err) => JsonResponse::from_error(err),
        }
    }
}

impl DispatchTable {
    pub fn new() -> DispatchTable {
        DispatchTable {
            api: API {
                version: "1.0.0".into(),
                modules: vec![],
            },
            sync_runners: HashMap::new(),
            #[cfg(feature = "node_interaction")]
            async_runners: HashMap::new(),
        }
    }

    pub fn register_api_function<P, R>(
        &mut self,
        module: &mut Module,
        info: fn() -> api_doc::api::Function,
    ) -> String
    where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
    {
        module.types.push(P::type_info());
        module.types.push(R::type_info());
        let function = info();
        let name = format!("{}.{}", module.name, function.name);
        module.functions.push(function);
        name
    }

    pub fn register_api_function_no_args<R>(
        &mut self,
        module: &mut Module,
        info: fn() -> api_doc::api::Function,
    ) -> String
    where
        R: TypeInfo + Send + Serialize + 'static,
    {
        module.types.push(R::type_info());
        let function = info();
        let name = format!("{}.{}", module.name, function.name);
        module.functions.push(function);
        name
    }

    pub fn call<P, R>(
        &mut self,
        name: &str,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> ApiResult<R>,
    ) where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
    {
        self.sync_runners
            .insert(name.into(), Box::new(CallHandler::new(handler)));

        #[cfg(feature = "node_interaction")]
        self.async_runners.insert(
            name.into(),
            Box::new(SpawnHandler::new(move |context, params| async move {
                handler(context, params)
            })),
        );
    }

    pub fn call_no_api<P, R>(
        &mut self,
        name: &str,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> ApiResult<R>,
    ) where
        P: Send + DeserializeOwned + 'static,
        R: Send + Serialize + 'static,
    {
        self.sync_runners
            .insert(name.into(), Box::new(CallHandler::new(handler)));

        #[cfg(feature = "node_interaction")]
        self.async_runners.insert(
            name.into(),
            Box::new(SpawnHandler::new(move |context, params| async move {
                handler(context, params)
            })),
        );
    }

    pub fn call_no_args<R>(
        &mut self,
        name: &str,
        handler: fn(context: std::sync::Arc<ClientContext>) -> ApiResult<R>,
    ) where
        R: TypeInfo + Send + Serialize + 'static,
    {
        self.sync_runners
            .insert(name.into(), Box::new(CallNoArgsHandler::new(handler)));

        #[cfg(feature = "node_interaction")]
        self.async_runners.insert(
            name.into(),
            Box::new(SpawnNoArgsHandler::new(move |context| async move {
                handler(context)
            })),
        );
    }

    #[cfg(feature = "node_interaction")]
    pub fn spawn<P, R, F>(
        &mut self,
        name: &str,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> F,
    ) where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
        F: Send + Future<Output = ApiResult<R>> + 'static,
    {
        self.async_runners
            .insert(name.into(), Box::new(SpawnHandler::new(handler)));

        self.sync_runners.insert(
            name.into(),
            Box::new(CallHandler::new(move |context, params| {
                context
                    .clone()
                    .async_runtime_handle
                    .block_on(handler(context, params))
            })),
        );
    }

    pub fn register_sync<P, R>(
        &mut self,
        module: &mut Module,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> ApiResult<R>,
        info: fn() -> api_doc::api::Function,
    ) where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
    {
        let name = self.register_api_function::<P, R>(module, info);
        self.sync_runners
            .insert(name.clone(), Box::new(CallHandler::new(handler)));

        #[cfg(feature = "node_interaction")]
        self.async_runners.insert(
            name.clone(),
            Box::new(SpawnHandler::new(move |context, params| async move {
                handler(context, params)
            })),
        );
    }

    pub fn register_sync_no_args<R>(
        &mut self,
        module: &mut Module,
        handler: fn(context: std::sync::Arc<ClientContext>) -> ApiResult<R>,
        info: fn() -> api_doc::api::Function,
    ) where
        R: TypeInfo + Send + Serialize + 'static,
    {
        let name = self.register_api_function_no_args::<R>(module, info);
        self.sync_runners
            .insert(name.clone(), Box::new(CallNoArgsHandler::new(handler)));

        #[cfg(feature = "node_interaction")]
        self.async_runners.insert(
            name.clone(),
            Box::new(SpawnNoArgsHandler::new(move |context| async move {
                handler(context)
            })),
        );
    }

    pub fn register_async<P, R, F>(
        &mut self,
        module: &mut Module,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> F,
        info: fn() -> api_doc::api::Function,
    ) where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
        F: Send + Future<Output = ApiResult<R>> + 'static,
    {
        let name = self.register_api_function::<P, R>(module, info);
        self.async_runners
            .insert(name.clone(), Box::new(SpawnHandler::new(handler)));

        self.sync_runners.insert(
            name,
            Box::new(CallHandler::new(move |context, params| {
                context
                    .clone()
                    .async_runtime_handle
                    .block_on(handler(context, params))
            })),
        );
    }

    #[cfg(feature = "node_interaction")]
    pub fn spawn_no_api<P, R, F>(
        &mut self,
        name: &str,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> F,
    ) where
        P: Send + DeserializeOwned + 'static,
        R: Send + Serialize + 'static,
        F: Send + Future<Output = ApiResult<R>> + 'static,
    {
        self.async_runners
            .insert(name.into(), Box::new(SpawnHandler::new(handler)));

        self.sync_runners.insert(
            name.into(),
            Box::new(CallHandler::new(
                move |context: std::sync::Arc<ClientContext>, params: P| -> ApiResult<R> {
                    context
                        .clone()
                        .async_runtime_handle
                        .block_on(handler(context, params))
                },
            )),
        );
    }

    pub fn call_raw_async(
        &mut self,
        name: &str,
        handler: fn(
            context: std::sync::Arc<ClientContext>,
            params_json: String,
            request_id: u32,
            on_result: Box<Callback>,
        ),
    ) {
        self.async_runners
            .insert(name.into(), Box::new(RawAsyncHandler::new(handler)));
    }

    pub fn sync_dispatch(
        &self,
        context: std::sync::Arc<ClientContext>,
        name: String,
        params_json: String,
    ) -> JsonResponse {
        match self.sync_runners.get(&name) {
            Some(handler) => handler.handle(context, params_json.as_str()),
            None => JsonResponse::from_error(ApiError::unknown_function(&name)),
        }
    }

    #[cfg(feature = "node_interaction")]
    pub fn async_dispatch(
        &self,
        context: std::sync::Arc<ClientContext>,
        function: String,
        params_json: String,
        request_id: u32,
        on_result: Box<Callback>,
    ) {
        match self.async_runners.get(&function) {
            Some(handler) => handler.handle(context, params_json, request_id, on_result),
            None => JsonResponse::from_error(ApiError::unknown_function(&function)).send(
                &*on_result,
                request_id,
                1,
            ),
        }
    }

    #[cfg(not(feature = "node_interaction"))]
    pub fn async_dispatch(
        &self,
        context: std::sync::Arc<ClientContext>,
        function: String,
        params_json: String,
        request_id: u32,
        on_result: Box<Callback>,
    ) {
        self.sync_dispatch(context, function, params_json)
            .send(&on_result, request_id, 1);
    }

    pub(crate) fn register_module<'h, M: TypeInfo>(
        &'h mut self,
        types: &[fn() -> Field],
        functions: fn(register: &mut Registrar<'h>) -> (),
    ) {
        let mut registrar = Registrar::<'h> {
            dispatcher: self,
            module: Module::new::<M>(types),
        };
        functions(&mut registrar);
        registrar.dispatcher.api.modules.push(registrar.module);
    }
}

pub(crate) struct Registrar<'a> {
    module: Module,
    dispatcher: &'a mut DispatchTable,
}

impl Registrar<'_> {
    pub fn async_func<P, R, F>(
        &mut self,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> F,
        info: fn() -> api_doc::api::Function,
    ) where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
        F: Send + Future<Output = ApiResult<R>> + 'static,
    {
        self.dispatcher
            .register_async(&mut self.module, handler, info);
    }

    pub fn func<P, R>(
        &mut self,
        handler: fn(context: std::sync::Arc<ClientContext>, params: P) -> ApiResult<R>,
        info: fn() -> api_doc::api::Function,
    ) where
        P: TypeInfo + Send + DeserializeOwned + 'static,
        R: TypeInfo + Send + Serialize + 'static,
    {
        self.dispatcher
            .register_sync(&mut self.module, handler, info);
    }

    pub fn func_no_args<R>(
        &mut self,
        handler: fn(context: std::sync::Arc<ClientContext>) -> ApiResult<R>,
        info: fn() -> api_doc::api::Function,
    ) where
        R: TypeInfo + Send + Serialize + 'static,
    {
        self.dispatcher
            .register_sync_no_args(&mut self.module, handler, info);
    }
}
