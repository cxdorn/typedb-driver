/*
 * Copyright (C) 2021 Vaticle
 *
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use futures::executor;
use log::warn;
use typedb_protocol::Options;
use typedb_protocol::session as session_proto;
use crate::common::error::MESSAGES;

use crate::common::Result;
use crate::rpc::builder::session::{close_req, open_req};
use crate::rpc::client::RpcClient;
use crate::transaction;
use crate::transaction::Transaction;

#[derive(Copy, Clone, Debug)]
pub enum Type {
    Data = 0,
    Schema = 1
}

impl Type {
    fn to_proto(&self) -> session_proto::Type {
        match self {
            Type::Data => session_proto::Type::Data,
            Type::Schema => session_proto::Type::Schema
        }
    }
}

#[derive(Debug)]
pub struct Session {
    pub db_name: String,
    pub(crate) id: Vec<u8>,
    pub(crate) rpc_client: RpcClient,
    is_open_atomic: AtomicBool,
    network_latency: Duration,
    session_type: Type,
}

impl Session {
    pub(crate) async fn new(db_name: &str, session_type: Type, rpc_client: &RpcClient) -> Result<Self> {
        let start_time = Instant::now();
        let open_req = open_req(db_name, session_type.to_proto(), None);
        let mut rpc_client_clone = rpc_client.clone();
        let res = rpc_client_clone.session_open(open_req).await?;
        // TODO: pulse task
        Ok(Session {
            db_name: String::from(db_name),
            session_type,
            network_latency: Self::compute_network_latency(start_time, res.server_duration_millis),
            id: res.session_id,
            rpc_client: rpc_client_clone,
            is_open_atomic: AtomicBool::new(true)
        })
    }

    pub async fn transaction(&self, transaction_type: transaction::Type) -> Result<Transaction> {
        match self.is_open() {
            true => Transaction::new(&self.id, transaction_type, self.network_latency, &self.rpc_client).await,
            false => Err(MESSAGES.client.session_is_closed.to_err(vec![]))
        }
    }

    pub fn get_type(&self) -> Type {
        self.session_type
    }

    fn is_open(&self) -> bool {
        self.is_open_atomic.load(Ordering::Relaxed)
    }

    pub async fn close(&mut self) {
        if let Ok(true) = self.is_open_atomic.compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed) {
            let res = self.rpc_client.session_close(close_req(self.id.clone())).await;
            // TODO: the request errors harmlessly if the session is already closed. Protocol should
            //       expose the cause of the error and we can use that to decide whether to warn here.
            if res.is_err() { warn!("{}", MESSAGES.client.session_close_failed.to_err(vec![])) }
        }
    }

    fn compute_network_latency(start_time: Instant, server_duration_millis: i32) -> Duration {
        Duration::from_millis((Instant::now() - start_time).as_millis() as u64 - server_duration_millis as u64)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // TODO: refactor to hand off the closure to a background process
        assert!(!self.is_open(), "{}", MESSAGES.client.session_was_never_closed.to_err(vec![]));
        if self.is_open() {
            warn!("{}", MESSAGES.client.session_was_never_closed.to_err(vec![]))
        }
        // let mut rpc_client = self.rpc_client.clone();
        // let id = self.id.clone();
        // let closer = tokio::join!(async move {
        //     // println!("banana");
        //     let res = rpc_client.session_close(close_req(id)).await;
        //     if res.is_err() { warn!("{}", MESSAGES.client.session_close_failed.to_err(vec![])) }
        // });
        // // executor::block_on(closer).expect("FAIL");
    }
}
