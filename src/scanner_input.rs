use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannerInput {
    Wifi(SocketAddr),
    Usb,
}

impl ScannerInput {
    pub fn network_address(self) -> Result<SocketAddr, ScannerInputError> {
        match self {
            Self::Wifi(address) => Ok(address),
            Self::Usb => Err(ScannerInputError::UsbBackendNotImplemented),
        }
    }
}

impl FromStr for ScannerInput {
    type Err = ScannerInputError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "usb" {
            return Ok(Self::Usb);
        }
        value
            .parse::<IpAddr>()
            .map(|ip| Self::Wifi(SocketAddr::new(ip, 80)))
            .map_err(|_| ScannerInputError::InvalidInput(value.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScannerInputError {
    InvalidInput(String),
    UsbBackendNotImplemented,
}

impl Display for ScannerInputError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(input) => {
                write!(
                    formatter,
                    "invalid scanner input {input:?}; expected an IP address or usb"
                )
            }
            Self::UsbBackendNotImplemented => formatter.write_str(
                "USB media acquisition is not implemented yet; use a scanner IP for Wi-Fi input",
            ),
        }
    }
}

impl Error for ScannerInputError {}
