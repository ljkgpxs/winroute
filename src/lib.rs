/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! # WinRoute
//! This crate is a utilities of high level of interface for manipulating
//! and observing Windows's routing table
//!
//! # Examples
//! ## Manage routing table
//! ```
//! let manager = RouteManager::new()?;
//! let new_route = Route::new("223.6.6.6".parse().unwrap(), 32).metric(1);
//! // add route
//! if let Err(e) = manager.add_route(&new_route) {
//!     eprintln!("{e}");
//! }
//! // delete route
//! if let Err(e) = manager.delete_route(&new_route) {
//!     eprintln!("{e}");
//! }
//! ```
//!
//! ## Listen a table change event
//! ```
//! let manager = RouteManager::new()?;
//! let recvier = manager.subscribe_route_change();
//! let ma = Arc::new(manager);
//! let mb = ma.clone();
//!
//! // start a thread to driven event loop, also can use async task to run this
//! std::thread::spawn(move || loop {
//!     ma.poll().unwrap();
//! });
//!
//! // create a new route
//! let new_route = Route::new("223.6.6.6".parse().unwrap(), 32);
//! // add route to system
//! mb.add_route(&new_route)?;
//!
//! loop {
//!     // listeing on route change event
//!     let event = recvier.recv().unwrap();
//!     println!("{:?}", event);
//! }
//! ```

mod manager;
mod route;
mod windows;

pub use manager::RouteEvent;
pub use manager::RouteManager;
pub use route::Route;

