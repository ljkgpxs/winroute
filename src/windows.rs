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

use std::{io, net::IpAddr};

use crossbeam_channel::Sender;
use winapi::{
    shared::{
        netioapi::*,
        nldef::MIB_IPPROTO_NETMGMT,
        ntdef::{BOOLEAN, HANDLE, PVOID},
        ws2def::{AF_INET, AF_INET6, AF_UNSPEC, PSOCKADDR, SOCKADDR_IN},
        ws2ipdef::SOCKADDR_IN6,
    },
    um::iphlpapi::GetBestInterfaceEx,
};

use crate::{manager::SystemRouteOperate, Route, RouteEvent};

pub(crate) struct WindowsOperator {
    notify_handle: Option<HANDLE>,
    sender: Sender<RouteEvent>,
}

impl WindowsOperator {
    fn register_route_listener(&self) -> io::Result<()> {
        if let Some(_) = self.notify_handle {
            return Err(code_to_error(5010, "Already registered"));
        } else {
            let mut handle = std::ptr::null_mut();
            let ret = unsafe {
                NotifyRouteChange2(
                    AF_UNSPEC as u16,
                    Some(callback),
                    std::mem::transmute(&self.sender),
                    BOOLEAN::from(false),
                    &mut handle,
                )
            };
            if ret != 0 {
                return Err(code_to_error(ret, "error notify route change"));
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
        if err != 0 {
            return Err(code_to_error(err, "error creating entry"));
        }
        Ok(())
    }

    fn delete_route(&self, route: &Route) -> io::Result<()> {
        let row: MIB_IPFORWARD_ROW2 = MIB_IPFORWARD_ROW2::from(route);

        let err = unsafe { DeleteIpForwardEntry2(&row) };
        if err != 0 {
            return Err(code_to_error(err, "error deleting entry"));
        }
        Ok(())
    }

    fn read_all_routes(&self) -> io::Result<Vec<Route>> {
        let mut ptable: PMIB_IPFORWARD_TABLE2 = std::ptr::null_mut();

        let ret = unsafe { GetIpForwardTable2(AF_UNSPEC as u16, &mut ptable) };
        if ret != 0 {
            return Err(code_to_error(ret, "Error getting table"));
        }

        let prows = unsafe {
            std::ptr::slice_from_raw_parts(
                &(*ptable).Table as *const MIB_IPFORWARD_ROW2,
                (*ptable).NumEntries as usize,
            )
        };

        let entries = unsafe { (*ptable).NumEntries };
        let res = (0..entries)
            .map(|idx| unsafe { (*prows)[idx as usize] })
            .filter_map(|row| Some(Route::from(&row)))
            .collect();
        unsafe { FreeMibTable(ptable as *mut _) };
        Ok(res)
    }

    fn init(&self) -> io::Result<()> {
        self.register_route_listener()?;
        Ok(())
    }

    fn new(sender: Sender<RouteEvent>) -> Self
    where
        Self: Sized,
    {
        Self {
            notify_handle: None,
            sender,
        }
    }
}

impl Drop for WindowsOperator {
    fn drop(&mut self) {
        if let Some(handle) = self.notify_handle {
            unsafe {
                CancelMibChangeNotify2(handle);
            }
        }
    }
}

impl From<&MIB_IPFORWARD_ROW2> for Route {
    fn from(row: &MIB_IPFORWARD_ROW2) -> Self {
        let dst_family = unsafe { (*row).DestinationPrefix.Prefix.si_family() };
        let dst = unsafe {
            match *dst_family as i32 {
                AF_INET => IpAddr::from(std::mem::transmute::<_, [u8; 4]>(
                    (*row).DestinationPrefix.Prefix.Ipv4().sin_addr,
                )),
                AF_INET6 => IpAddr::from(std::mem::transmute::<_, [u8; 16]>(
                    (*row).DestinationPrefix.Prefix.Ipv6().sin6_addr,
                )),
                _ => panic!("Unexpected family {}", dst_family),
            }
        };

        let dst_len = (*row).DestinationPrefix.PrefixLength;

        let nexthop_family = unsafe { (*row).NextHop.si_family() };

        let gateway = unsafe {
            match *nexthop_family as i32 {
                AF_INET => IpAddr::from(std::mem::transmute::<_, [u8; 4]>(
                    (*row).NextHop.Ipv4().sin_addr,
                )),
                AF_INET6 => IpAddr::from(std::mem::transmute::<_, [u8; 16]>(
                    (*row).NextHop.Ipv6().sin6_addr,
                )),
                _ => panic!("Unexpected family {}", dst_family),
            }
        };

        let mut route = Route::new(dst, dst_len)
            .ifindex((*row).InterfaceIndex)
            .luid(unsafe { std::mem::transmute((*row).InterfaceLuid) })
            .metric((*row).Metric);

        route.gateway = gateway;
        route
    }
}

impl From<&Route> for MIB_IPFORWARD_ROW2 {
    fn from(route: &Route) -> Self {
        let mut row: MIB_IPFORWARD_ROW2 = unsafe { std::mem::zeroed() };
        unsafe { InitializeIpForwardEntry(&mut row) };

        if let Some(ifindex) = route.ifindex {
            row.InterfaceIndex = ifindex;
        }

        if let Some(luid) = route.luid {
            row.InterfaceLuid = unsafe { std::mem::transmute(luid) };
        }

        match route.gateway {
            IpAddr::V4(addr) => unsafe {
                *row.NextHop.si_family_mut() = AF_INET as u16;
                row.NextHop.Ipv4_mut().sin_addr = std::mem::transmute(addr.octets());
            },
            IpAddr::V6(addr) => unsafe {
                *row.NextHop.si_family_mut() = AF_INET as u16;
                row.NextHop.Ipv6_mut().sin6_addr = std::mem::transmute(addr.octets());
            },
        }

        row.DestinationPrefix.PrefixLength = route.prefix;
        match route.destination {
            IpAddr::V4(addr) => unsafe {
                *row.DestinationPrefix.Prefix.si_family_mut() = AF_INET as u16;
                row.DestinationPrefix.Prefix.Ipv4_mut().sin_addr =
                    std::mem::transmute(addr.octets());
            },
            IpAddr::V6(addr) => unsafe {
                *row.DestinationPrefix.Prefix.si_family_mut() = AF_INET6 as u16;
                row.DestinationPrefix.Prefix.Ipv6_mut().sin6_addr =
                    std::mem::transmute(addr.octets());
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

unsafe extern "system" fn callback(
    callercontext: PVOID,
    row: PMIB_IPFORWARD_ROW2,
    notification_type: MIB_NOTIFICATION_TYPE,
) {
    // let tx = &*(callercontext as *const broadcast::Sender<RouteChange>);
    let route = Route::from(&*row);
    let sender: &Sender<RouteEvent> = std::mem::transmute(callercontext);
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
            let mut addr: SOCKADDR_IN = unsafe { std::mem::zeroed() };
            addr.sin_family = AF_INET as u16;
            addr.sin_addr = unsafe { std::mem::transmute(v4.octets()) };
            let ptr: PSOCKADDR = unsafe { std::mem::transmute(&mut addr) };
            unsafe { GetBestInterfaceEx(ptr, &mut result as *mut _) }
        }
        IpAddr::V6(v6) => {
            let mut addr: SOCKADDR_IN6 = unsafe { std::mem::zeroed() };
            addr.sin6_family = AF_INET6 as u16;
            addr.sin6_addr = unsafe { std::mem::transmute(v6.octets()) };
            let ptr: PSOCKADDR = unsafe { std::mem::transmute(&addr) };
            let rp = result as *mut u32;
            unsafe {
                (*rp) = 100;
            }
            unsafe { GetBestInterfaceEx(ptr, result as *mut _) }
        }
    };

    if ret != 0 {
        return Err(code_to_error(ret, "Failed to get best interface"));
    }

    Ok(result)
}

#[cfg(test)]
pub mod test_cast {
    use winapi::shared::{netioapi::MIB_IPFORWARD_ROW2, nldef::MIB_IPPROTO_NETMGMT};

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
