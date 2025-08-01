use url::Url;

#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub enum ConnectedPoint {
    Dialer { addr: Url },
    Listener { local_addr: Url, remote_addr: Url },
}

impl ConnectedPoint {
    pub fn to_endpoint(&self) -> Endpoint {
        match self {
            ConnectedPoint::Dialer { .. } => Endpoint::Dialer,
            ConnectedPoint::Listener { .. } => Endpoint::Listener,
        }
    }

    pub fn is_dialer(&self) -> bool {
        matches!(self, ConnectedPoint::Dialer { .. })
    }

    pub fn is_listener(&self) -> bool {
        matches!(self, ConnectedPoint::Listener { .. })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Endpoint {
    Dialer,
    Listener,
}

impl std::ops::Not for Endpoint {
    type Output = Endpoint;

    fn not(self) -> Self::Output {
        match self {
            Endpoint::Dialer => Endpoint::Listener,
            Endpoint::Listener => Endpoint::Dialer,
        }
    }
}

impl Endpoint {
    pub fn is_dialer(self) -> bool {
        matches!(self, Endpoint::Dialer)
    }

    pub fn is_listener(self) -> bool {
        matches!(self, Endpoint::Listener)
    }
}

impl From<&'_ ConnectedPoint> for Endpoint {
    fn from(endpoint: &'_ ConnectedPoint) -> Endpoint {
        endpoint.to_endpoint()
    }
}

impl From<ConnectedPoint> for Endpoint {
    fn from(endpoint: ConnectedPoint) -> Endpoint {
        endpoint.to_endpoint()
    }
}
