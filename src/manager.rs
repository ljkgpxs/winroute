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

use std::{
    cell::RefCell,
    error::Error,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Mutex, PoisonError},
};

use crossbeam_channel::{Receiver, Sender};

use crate::Route;

pub(crate) trait SystemRouteOperate {
    fn new(sender: Sender<RouteEvent>) -> Self
    where
        Self: Sized;
    fn init(&mut self) -> io::Result<()>;
    fn read_all_routes(&self) -> io::Result<Vec<Route>>;
    fn add_route(&self, route: &Route) -> io::Result<()>;
    fn delete_route(&self, route: &Route) -> io::Result<()>;
}

/// Routing table change event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteEvent {
    Add(Route),
    Delete(Route),
    Change(Route),
}

/// Route manager structure, using ```RouteManager::new()``` to create a new one
/// 
/// # Examples
///
/// ```rust no_run
/// use winroute::*;
/// fn main() -> std::io::Result<()> {
///     let manager = RouteManager::new()?;
///     let routes = manager.routes()?;
///     println!("Routes count: {}", routes.len());
///     for r in routes {
///         println!("{r}");
///     }
///     Ok(())
/// }
/// ```
/// 
pub struct RouteManager {
    routes: Mutex<RefCell<Vec<Route>>>,
    operator: Box<dyn SystemRouteOperate>,
    operator_receiver: Receiver<RouteEvent>,
    subscribers: Receiver<RouteEvent>,
    producer: Sender<RouteEvent>,
}

impl RouteManager {
    /// Create a RouteManager
    ///
    /// # Errors
    /// When windows NotifyRouteChange2 return error will panic
    #[cfg(windows)]
    pub fn new() -> io::Result<Self> {
        use crate::windows::WindowsOperator;

        let (tx, rx) = crossbeam_channel::unbounded();
        let (tx_loop, rx_loop) = crossbeam_channel::unbounded();
        let mut operator = Box::new(WindowsOperator::new(tx));
        operator.init()?;
        let routes = operator.read_all_routes().unwrap();

        let manager = RouteManager {
            routes: Mutex::new(RefCell::new(routes)),
            operator,
            operator_receiver: rx,
            subscribers: rx_loop,
            producer: tx_loop,
        };

        Ok(manager)
    }

    #[cfg(not(windows))]
    pub fn new() -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Other, "None windows system not supported"))
    }

    /// Driven subscribe event, you should run in separate thread or async task
    /// # Examples
    ///
    /// ```rust ignore
    /// use std::sync::Arc;
    /// use winroute::{Route, RouteManager};
    /// 
    /// let manager = Arc::new(RouteManager::new());
    /// let poll = manager.clone();
    /// ```
    ///
    /// ```rust ignore
    /// std::thread::spawn(move || loop {
    ///    poll.poll();
    /// });
    /// ```
    /// or
    /// ```rust ignore
    /// tokio::spawn(async move {
    ///     loop {
    ///         poll.poll();
    ///     }
    /// });
    /// ```
    ///
    /// # Errors
    /// When Mutex return error while invoke lock() or channel producer send data occurs error
    pub fn poll(&self) -> Result<(), Box<dyn Error>> {
        let event: RouteEvent = self.operator_receiver.recv()?;
        {
            match self.routes.lock() { Ok(guard) => {
                let mut routes = guard.borrow_mut();
                match event.clone() {
                    RouteEvent::Add(route) => routes.push(route),
                    RouteEvent::Delete(route) => {
                        if let Some(index) = routes.iter().position(|v| *v == route) {
                            routes.remove(index);
                        }
                    }
                    RouteEvent::Change(route) => {
                        if let Some(index) = routes.iter().position(|v| {
                            v.destination == route.destination && v.prefix == route.prefix
                        }) {
                            routes.remove(index);
                            routes.push(route);
                        }
                    }
                }
            } _ => {
                return Err(Box::new(PoisonError::new(
                    "Can not lock private field routes",
                )));
            }}
        }
        if let Err(e) = self.producer.send(event.clone()) {
            return Err(Box::new(e));
        }
        Ok(())
    }

    /// Subscribe routing table change event
    ///
    /// Return a Receiver, use .recv() method to receive RouteEvent
    pub fn subscribe_route_change(&self) -> Receiver<RouteEvent> {
        self.subscribers.clone()
    }

    /// Get system routing table, include IPv6 and IPv4 routes
    ///
    /// # Errors
    /// When try to lock Mutex and it return an error
    pub fn routes(&self) -> io::Result<Vec<Route>> {
        match self.routes.lock() { Ok(guard) => {
            Ok(guard.borrow_mut().clone())
        } _ => {
            Err(io::Error::new(io::ErrorKind::Other, "Can not lock inner data, this is a thread safe error"))
        }}
    }

    /// Add a new route to system's routing table
    ///
    /// # NOTICE
    ///
    /// if ```add_route``` is called by a user that is not a administrator or root, the function will fail and return ERROR_ACCESS_DENIED
    ///
    /// # Errors
    /// when system api return error
    pub fn add_route(&self, route: &Route) -> io::Result<()> {
        self.operator.add_route(route)?;
        Ok(())
    }

    /// Remove route from system's routing table
    ///
    /// # NOTICE
    ///
    /// if ```delete_route``` is called by a user that is not a administrator or root, the function will fail and return ERROR_ACCESS_DENIED
    ///
    /// # Errors
    /// when system api return error
    pub fn delete_route(&self, route: &Route) -> io::Result<()> {
        self.operator.delete_route(route)?;
        Ok(())
    }

    /// return default route
    /// 
    /// # Errors
    /// When try to lock Mutex and it return an error
    pub fn default_route(&self) -> io::Result<Option<Route>> {
        match self.routes.lock() { Ok(guard) => {
            let guard = guard.borrow_mut();
            let itr = guard.iter();
            for route in itr {
                if (route.destination == Ipv4Addr::UNSPECIFIED
                    || route.destination == Ipv6Addr::UNSPECIFIED)
                    && route.gateway != IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                    && route.gateway != IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                    && route.prefix == 0
                {
                    return Ok(Some(route.clone()));
                }
            }
        } _ => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "can not found defualt route",
            ));
        }}
        Ok(None)
    }
}

impl Drop for RouteManager {
    fn drop(&mut self) {}
}

unsafe impl Sync for RouteManager {}

unsafe impl Send for RouteManager {}
