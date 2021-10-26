use core::cmp;
use core::fmt;
use core::mem;

use esp_idf_sys::c_types::*;
use esp_idf_sys::*;

use crate::private::cstr;

pub type Result<T> = core::result::Result<T, LwIPError>;

pub enum AddressFamily {
	Ipv4,
	Ipv6,
}

impl Into<c_int> for AddressFamily {
	fn into(self) -> c_int {
		match self {
			AddressFamily::Ipv4 => AF_INET as _,
			AddressFamily::Ipv6 => AF_INET6 as _,
		}
	}
}

pub enum SocketType {
	Stream,
	Dgram,
	Raw,
}

impl Into<c_int> for SocketType {
	fn into(self) -> c_int {
		match self {
			SocketType::Stream => SOCK_STREAM as _,
			SocketType::Dgram => SOCK_DGRAM as _,
			SocketType::Raw => SOCK_RAW as _,
		}
	}
}

pub enum Protocol {
	Ip,
	Icmp,
	Tcp,
	Udp,
	Ipv6,
	Icmpv6,
	UdpLite,
	Raw,
}

impl Into<c_int> for Protocol {
	fn into(self) -> c_int {
		match self {
			Protocol::Ip => IPPROTO_IP as _,
			Protocol::Icmp => IPPROTO_ICMP as _,
			Protocol::Tcp => IPPROTO_TCP as _,
			Protocol::Udp => IPPROTO_UDP as _,
			Protocol::Ipv6 => IPPROTO_IPV6 as _,
			Protocol::Icmpv6 => IPPROTO_ICMPV6 as _,
			Protocol::UdpLite => IPPROTO_UDPLITE as _,
			Protocol::Raw => IPPROTO_RAW as _,
		}
	}
}

pub struct SocketAddrV4 {
	inner: sockaddr_in,
}

impl fmt::Display for SocketAddrV4 {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
		let mut addr_str = [0u8; INET_ADDRSTRLEN as usize];

		unsafe {
			ip4addr_ntoa_r(
				&self.inner.sin_addr as *const _ as _,
				addr_str.as_mut_ptr() as _,
				(addr_str.len() - 1) as _,
			)
		};

		let s = cstr::from_cstr(&addr_str);

		write!(f, "{}:{}", s, self.inner.sin_port)
	}
}

pub struct SocketAddrV6 {
	inner: sockaddr_in6,
}

impl fmt::Display for SocketAddrV6 {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
		let mut addr_str = [0u8; INET6_ADDRSTRLEN as usize];

		unsafe {
			ip6addr_ntoa_r(
				&self.inner.sin6_addr as *const _ as _,
				addr_str.as_mut_ptr() as _,
				(addr_str.len() - 1) as _,
			)
		};

		let s = cstr::from_cstr(&addr_str);

		write!(f, "[{}]:{}", s, self.inner.sin6_port)
	}
}

pub enum SocketAddr {
	V4(SocketAddrV4),
	V6(SocketAddrV6),
}

impl SocketAddr {
	pub fn inner(&self) -> (*const sockaddr, socklen_t) {
		match *self {
			SocketAddr::V4(ref a) => {
				(a as *const _ as *const _, mem::size_of_val(a) as socklen_t)
			}
			SocketAddr::V6(ref a) => {
				(a as *const _ as *const _, mem::size_of_val(a) as socklen_t)
			}
		}
	}
}

impl fmt::Display for SocketAddr {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SocketAddr::V4(ref a) => a.fmt(f),
			SocketAddr::V6(ref a) => a.fmt(f),
		}
	}
}

pub struct Socket(c_int);

impl Socket {
	pub fn open(family: AddressFamily, ty: SocketType, protocol: Protocol) -> Result<Self> {
		let raw = cvt(unsafe { lwip_socket(family.into(), ty.into(), protocol.into()) })?;

		Ok(Socket(raw))
	}

	pub fn bind(&mut self, addr: *const sockaddr) -> Result<()> {
		lwip!(unsafe { lwip_bind(self.0, addr, mem::size_of::<sockaddr>() as _) })
	}

	pub fn close(&mut self) -> Result<()> {
		lwip!(unsafe { lwip_close(self.0) })
	}

	pub fn shutdown(&mut self) -> Result<()> {
		lwip!(unsafe { lwip_shutdown(self.0, 0) })
	}

	pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
		let mut storage: sockaddr_storage = unsafe { mem::zeroed() };
		let mut addrlen = mem::size_of_val(&storage) as socklen_t;

		let n = cvt(unsafe {
			lwip_recvfrom(
				self.0,
				buf.as_mut_ptr() as *mut c_void,
				buf.len() as _,
				0,
				&mut storage as *mut _ as *mut _,
				&mut addrlen,
			)
		})?;

		Ok((n as usize, sockaddr_to_addr(&storage, addrlen as usize)?))
	}

	pub fn send_to(&self, buf: &mut [u8], dst: SocketAddr) -> Result<usize> {
		let len = cmp::min(buf.len(), <size_t>::MAX as usize) as size_t;
		let (dstp, dstlen) = dst.inner();

		let n = cvt(unsafe {
			lwip_sendto(self.0, buf.as_ptr() as *const c_void, len, 0, dstp, dstlen)
		})?;
		Ok(n as usize)
	}
}

impl Drop for Socket {
	fn drop(&mut self) {
		self.shutdown().ok();
		self.close().ok();
	}
}

fn cvt(v: c_int) -> Result<c_int> {
	lwip_result!(v, v)
}

fn sockaddr_to_addr(storage: &sockaddr_storage, len: usize) -> Result<SocketAddr> {
	match storage.ss_family as u32 {
		AF_INET => {
			assert!(len as usize >= mem::size_of::<sockaddr_in>());
			Ok(SocketAddr::V4(SocketAddrV4 {
				inner: unsafe { *(storage as *const _ as *const sockaddr_in) },
			}))
		}
		AF_INET6 => {
			assert!(len as usize >= mem::size_of::<sockaddr_in6>());
			Ok(SocketAddr::V6(SocketAddrV6 {
				inner: unsafe { *(storage as *const _ as *const sockaddr_in6) },
			}))
		}
		_ => Err(LwIPError::from_raw(err_enum_t_ERR_VAL)),
	}
}
