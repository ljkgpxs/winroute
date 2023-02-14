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
    fmt::Display,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

/// Routing data structure, including destination address, gateway and other information
#[cfg_attr(feature = "serializable", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// Network address of the destination. `0.0.0.0` with a prefix of `0` is considered a default route.
    pub destination: IpAddr,

    /// Prefix for the destination IP address of this route.
    pub prefix: u8,

    /// The address of the next hop
    pub gateway: IpAddr,

    /// The local index value for the network interface associated with this IP route entry.
    pub ifindex: Option<u32>,

    /// The route metric offset value for this IP route entry.
    pub metric: Option<u32>,

    /// The locally unique identifier (LUID) for the network interface associated with this IP route entry.
    pub luid: Option<u64>,

    /// The IP version number, the value is 4 or 6
    pub version: u8,
}

impl Route {
    /// Create a route that matches a given destination network.
    ///
    /// Either the gateway or interface should be set before attempting to add to a routing table.
    pub fn new(destination: IpAddr, prefix: u8) -> Self {
        let version = match destination {
            IpAddr::V4(_) => 4,
            IpAddr::V6(_) => 6,
        };
        Self {
            destination,
            prefix,
            gateway: match destination {
                IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            },
            ifindex: None,
            metric: None,
            luid: None,
            version,
        }
    }

    /// destination setter
    pub fn destination(mut self, destination: IpAddr) -> Self {
        self.destination = destination;
        self.version = match destination {
            IpAddr::V4(_) => 4,
            IpAddr::V6(_) => 6,
        };
        self
    }

    /// prefix setter
    pub fn prefix(mut self, prefix: u8) -> Self {
        self.prefix = prefix;
        self
    }

    /// gateway setter
    pub fn gateway(mut self, gateway: IpAddr) -> Self {
        self.gateway = gateway;
        self
    }

    /// ifindex setter
    pub fn ifindex(mut self, idx: u32) -> Self {
        self.ifindex = Some(idx);
        self
    }

    /// metric setter
    pub fn metric(mut self, metric: u32) -> Self {
        self.metric = Some(metric);
        self
    }

    /// luic setter
    pub fn luid(mut self, luid: u64) -> Self {
        self.luid = Some(luid);
        self
    }
}

impl Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} gateway {} metric {:?}",
            self.destination.to_string(),
            self.prefix,
            self.gateway.to_string(),
            self.metric,
        )
    }
}

#[cfg(feature = "serializable")]
struct CustomVisitor;

#[cfg(feature = "serializable")]
impl<'de> serde::Deserialize<'de> for Route {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(CustomVisitor)
    }
}
#[cfg(feature = "serializable")]
impl<'de> serde::de::Visitor<'de> for CustomVisitor {
    type Value = Route;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "a map with keys 'first' and 'second'")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut route = Route::new(IpAddr::V4("0.0.0.0".parse().unwrap()), 0);
        while let Some(key) = map.next_key()? {
            match key {
                "destination" => {
                    let dest_ip: String = map.next_value()?;
                    let res = dest_ip.parse::<IpAddr>();
                    if res.is_ok() {
                        route = route.destination(res.unwrap());
                    }
                }
                "prefix" => {
                    route = route.prefix(map.next_value()?);
                }
                "gateway" => {
                    let v = map.next_value();
                    if v.is_ok() {
                        route = route.gateway(v.unwrap());
                    }
                }
                "ifindex" => {
                    let v = map.next_value();
                    if v.is_ok() {
                        route = route.ifindex(v.unwrap());
                    }
                }
                "luid" => {
                    let v = map.next_value();
                    if v.is_ok() {
                        route = route.luid(v.unwrap());
                    }
                }
                "metric" => {
                    let v = map.next_value();
                    if v.is_ok() {
                        route = route.metric(v.unwrap());
                    }
                }
                _ => {
                    let _: serde::de::IgnoredAny = map.next_value()?;
                }
            }
        }
        Ok(route)
    }
}

#[cfg(test)]
pub mod test_route {
    use super::Route;

    #[test]
    fn testv4() {
        let route = Route::new("192.168.1.0".parse().unwrap(), 32)
            .destination("192.168.0.0".parse().unwrap())
            .prefix(24)
            .gateway("172.1.1.254".parse().unwrap())
            .ifindex(1)
            .luid(123456)
            .metric(1);
        assert_eq!(
            "192.168.0.0/24 gateway 172.1.1.254 metric Some(1)",
            route.to_string()
        );
        assert_eq!(4, route.version);

        let route = Route::new("192.168.1.0".parse().unwrap(), 32);
        assert_eq!(
            "192.168.1.0/32 gateway 0.0.0.0 metric None",
            route.to_string()
        );
    }

    #[test]
    fn testv6() {
        let route = Route::new("fe80:9464::".parse().unwrap(), 32);
        assert_eq!("fe80:9464::/32 gateway :: metric None", route.to_string());
        assert_eq!(6, route.version);
    }

    #[test]
    #[cfg(feature = "serializable")]
    fn test_serializable() {
        let route = Route::new("192.168.1.0".parse().unwrap(), 32)
            .destination("192.168.0.0".parse().unwrap())
            .prefix(24)
            .gateway("172.1.1.254".parse().unwrap())
            .ifindex(1)
            .luid(123456)
            .metric(1);
        let res = serde_json::to_string(&route).expect("Failed to serialize Route Object");
        assert_eq!("{\"destination\":\"192.168.0.0\",\"prefix\":24,\"gateway\":\"172.1.1.254\",\"ifindex\":1,\"metric\":1,\"luid\":123456,\"version\":4}", res);
        let route: Route = serde_json::from_str(&res).unwrap();
        assert_eq!(
            "192.168.0.0/24 gateway 172.1.1.254 metric Some(1)",
            route.to_string()
        );
        assert_eq!(4, route.version);

        let route = Route::new("fe80:9464::".parse().unwrap(), 32);
        let res = serde_json::to_string(&route).expect("Failed to serialize Route Object");
        assert_eq!("{\"destination\":\"fe80:9464::\",\"prefix\":32,\"gateway\":\"::\",\"ifindex\":null,\"metric\":null,\"luid\":null,\"version\":6}", res);
        let route: Route = serde_json::from_str(&res).unwrap();
        assert_eq!("fe80:9464::/32 gateway :: metric None", route.to_string());
        assert_eq!(6, route.version);
    }
}
