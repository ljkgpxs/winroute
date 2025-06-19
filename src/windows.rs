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

use std::{io, net::{IpAddr, Ipv4Addr, Ipv6Addr}, slice};

use crossbeam_channel::Sender;
use windows::Win32::{Foundation::HANDLE, NetworkManagement::{IpHelper::{CancelMibChangeNotify2, CreateIpForwardEntry2, DeleteIpForwardEntry2, FreeMibTable, GetBestInterfaceEx, GetIpForwardTable2, InitializeIpForwardEntry, MibAddInstance, MibDeleteInstance, MibParameterNotification, NotifyRouteChange2, MIB_IPFORWARD_ROW2, MIB_NOTIFICATION_TYPE}, Ndis::NET_LUID_LH}, Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, MIB_IPPROTO_NETMGMT, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6}};

use crate::{manager::SystemRouteOperate, Route, RouteEvent};

pub(crate) struct WindowsOperator {
    notify_handle: Option<HANDLE>,
    // ensure a constant memory address for callback fn
    sender: *mut Sender<RouteEvent>,
}

impl WindowsOperator {
    fn register_route_listener(&mut self) -> io::Result<()> {
        if let Some(_) = self.notify_handle {
            return Err(code_to_error(5010, "Already registered"));
        } else {
            let mut handle = HANDLE::default();
            let ret = unsafe {
                NotifyRouteChange2(
                    AF_UNSPEC,
                    Some(callback),
                    self.sender.cast(),
                    false,
                    &mut handle,
                )
            };
            if ret.is_err() {
                return Err(code_to_error(ret.0, "error notify route change"));
            }
            if !handle.is_invalid() {
                self.notify_handle = Some(handle)
            }
            Ok(())
        }
    }
}

impl SystemRouteOperate for WindowsOperator {
    fn add_route(&self, route: &Route) -> io::Result<()> {
        // if not set interface index and luid, it will use default route's params
        let row = if route.ifindex.is_none() && route.luid.is_none() {
            let best_idx = find_best_interface(route.gateway)?;
            let mut clone = route.clone();
            clone.ifindex = Some(best_idx);
            MIB_IPFORWARD_ROW2::from(&clone)
        } else {
            MIB_IPFORWARD_ROW2::from(route)
        };

        let err = unsafe { CreateIpForwardEntry2(&row) };
        if err.is_err() {
            return Err(code_to_error(err.0, "error creating entry"));
        }
        Ok(())
    }

    fn delete_route(&self, route: &Route) -> io::Result<()> {
        let row: MIB_IPFORWARD_ROW2 = MIB_IPFORWARD_ROW2::from(route);

        let err = unsafe { DeleteIpForwardEntry2(&row) };
        if err.is_err() {
            return Err(code_to_error(err.0, "error deleting entry"));
        }
        Ok(())
    }

    fn read_all_routes(&self) -> io::Result<Vec<Route>> {
        let mut ptable = std::ptr::null_mut();

        let ret = unsafe { GetIpForwardTable2(AF_UNSPEC, &mut ptable) };
        if ret.is_err() {
            return Err(code_to_error(ret.0, "Error getting table"));
        }

        let num_entries = usize::try_from(unsafe { *ptable }.NumEntries).unwrap();

        let rows = unsafe { slice::from_raw_parts((*ptable).Table.as_ptr(), num_entries) }.to_vec();

        let res = rows.iter().filter_map(|row| Some(Route::from(row))).collect();

        unsafe { FreeMibTable(ptable as *const _) };
        Ok(res)
    }

    fn init(&mut self) -> io::Result<()> {
        self.register_route_listener()?;
        Ok(())
    }

    fn new(sender: Sender<RouteEvent>) -> Self
    where
        Self: Sized,
    {
        Self {
            notify_handle: None,
            sender: Box::into_raw(Box::new(sender)),
        }
    }
}

impl Drop for WindowsOperator {
    fn drop(&mut self) {
        if let Some(handle) = self.notify_handle {
            unsafe {
                let _ = CancelMibChangeNotify2(handle);
            }
        }
        unsafe { drop(Box::from_raw(self.sender)) }
    }
}

impl From<&MIB_IPFORWARD_ROW2> for Route {
    fn from(row: &MIB_IPFORWARD_ROW2) -> Self {
        let dst_family = unsafe { row.DestinationPrefix.Prefix.si_family };
        let dst = unsafe {
            match dst_family {
                AF_INET => IpAddr::from(Ipv4Addr::from(row.DestinationPrefix.Prefix.Ipv4.sin_addr)),
                AF_INET6 => IpAddr::from(Ipv6Addr::from(row.DestinationPrefix.Prefix.Ipv6.sin6_addr)),
                _ => panic!("Unexpected family {:?}", dst_family),
            }
        };

        let dst_len = row.DestinationPrefix.PrefixLength;

        let nexthop_family = unsafe { row.NextHop.si_family };

        let gateway = unsafe {
            match nexthop_family {
                AF_INET => IpAddr::from(Ipv4Addr::from(row.NextHop.Ipv4.sin_addr)),
                AF_INET6 => IpAddr::from(Ipv6Addr::from(row.NextHop.Ipv6.sin6_addr)),
                _ => panic!("Unexpected family {:?}", dst_family),
            }
        };

        let mut route = Route::new(dst, dst_len)
            .ifindex(row.InterfaceIndex)
            .luid(unsafe { row.InterfaceLuid.Value })
            .metric(row.Metric);

        route.gateway = gateway;
        route
    }
}

impl From<&Route> for MIB_IPFORWARD_ROW2 {
    fn from(route: &Route) -> Self {
        let mut row: MIB_IPFORWARD_ROW2 = MIB_IPFORWARD_ROW2::default();
        unsafe { InitializeIpForwardEntry(&mut row) };

        if let Some(ifindex) = route.ifindex {
            row.InterfaceIndex = ifindex;
        }

        if let Some(luid) = route.luid {
            let mut api_luid = NET_LUID_LH::default();
            api_luid.Value = luid;
            row.InterfaceLuid = api_luid;
        }

        match route.gateway {
            IpAddr::V4(addr) => {
                row.NextHop.si_family = AF_INET;
                row.NextHop.Ipv4.sin_addr = addr.into();
            },
            IpAddr::V6(addr) => {
                row.NextHop.si_family = AF_INET;
                row.NextHop.Ipv6.sin6_addr = addr.into();
            },
        }

        row.DestinationPrefix.PrefixLength = route.prefix;
        match route.destination {
            IpAddr::V4(addr) => {
                row.DestinationPrefix.Prefix.si_family = AF_INET;
                row.DestinationPrefix.Prefix.Ipv4.sin_addr = addr.into();
            },
            IpAddr::V6(addr) => {
                row.DestinationPrefix.Prefix.si_family = AF_INET6;
                row.DestinationPrefix.Prefix.Ipv6.sin6_addr = addr.into();
            },
        }

        if let Some(metric) = route.metric {
            row.Metric = metric;
        } else {
            row.Metric = 0;
        }

        row.Protocol = MIB_IPPROTO_NETMGMT;

        row
    }
}

extern "system" fn callback(
    callercontext: *const core::ffi::c_void,
    row: *const MIB_IPFORWARD_ROW2,
    notification_type: MIB_NOTIFICATION_TYPE,
) {
    unsafe {
        let route = Route::from(&*row);
        if let Some(sender) = (callercontext as *const Sender<RouteEvent>).as_ref() {
            let event = match notification_type {
                n if n == MibParameterNotification => RouteEvent::Change(route),
                n if n == MibAddInstance => RouteEvent::Add(route),
                n if n == MibDeleteInstance => RouteEvent::Delete(route),
                _ => return,
            };
            if let Err(_) = sender.send(event) {
                // If there is no receiver, this may indicate that the system is currently shutting down
            }
        }
    }
}

fn code_to_error(code: u32, msg: &str) -> io::Error {
    let kind = match code {
        2 => io::ErrorKind::NotFound,
        5 => io::ErrorKind::PermissionDenied,
        87 => io::ErrorKind::InvalidInput,
        5010 => io::ErrorKind::AlreadyExists,
        1168 => io::ErrorKind::NotFound,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("{}: {}", msg, kind.to_string()))
}

pub fn find_best_interface(ip: IpAddr) -> io::Result<u32> {
    let mut result: u32 = 0;
    let ret = match ip {
        IpAddr::V4(v4) => {
            let mut addr = SOCKADDR_IN::default();
            addr.sin_family = AF_INET;
            addr.sin_addr = v4.into();
            unsafe { GetBestInterfaceEx(&addr as *const SOCKADDR_IN as *const SOCKADDR, &mut result as *mut _) }
        }
        IpAddr::V6(v6) => {
            let mut addr: SOCKADDR_IN6 = SOCKADDR_IN6::default();
            addr.sin6_family = AF_INET6;
            addr.sin6_addr = v6.into();
            unsafe { GetBestInterfaceEx(&addr as *const SOCKADDR_IN6 as *const SOCKADDR, result as *mut _) }
        }
    };

    if ret != 0 {
        return Err(code_to_error(ret, "Failed to get best interface"));
    }

    Ok(result)
}

#[cfg(test)]
pub mod test_cast {

    use windows::Win32::{NetworkManagement::IpHelper::MIB_IPFORWARD_ROW2, Networking::WinSock::MIB_IPPROTO_NETMGMT};

    use super::{find_best_interface, Route};

    #[test]
    fn cast_from_route() {
        let route = Route::new("192.168.1.0".parse().unwrap(), 24);
        let row = MIB_IPFORWARD_ROW2::from(&route);
        assert_eq!(0, row.Metric);
        assert_eq!(MIB_IPPROTO_NETMGMT, row.Protocol);
        assert_eq!("192.168.1.0", route.destination.to_string());
    }

    #[test]
    fn test_best_interface() {
        let idx = find_best_interface("192.168.1.1".parse().unwrap());
        assert_eq!(true, idx.is_ok());
    }
}
