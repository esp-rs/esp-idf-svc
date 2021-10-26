use core::mem;

use anyhow::*;
use log::*;

use esp_idf_sys::c_types::*;
use esp_idf_sys::*;

use crate::private::cstr::{CStr, CString};
use crate::task::{TaskConfig, TaskHandle};
use crate::lwip;

/// 0.0.0.0
const IPADDR_ANY: u32 = 0x00000000;

const DNS_PORT: c_ushort = 53;
const DNS_MAX_LEN: usize = 256;

const OPCODE_MASK: u16 = 0x7800;
const QR_FLAG: u16 = 1 << 7;
const QD_TYPE_A: u16 = 0x0001;
const ANS_TTL_SEC: u32 = 300;

pub struct CaptivePortalDns {
    task_handle: Option<TaskHandle>,
}

impl CaptivePortalDns {
    pub fn new() -> Self {
        CaptivePortalDns { task_handle: None }
    }

    pub fn start(&mut self) -> Result<()> {
        if self.task_handle.is_some() {
            bail!("dns server is already running");
        }

        let handle = TaskConfig::default()
            .priority(5)
            .spawn("dns_server", dns_server_task)?;

        self.task_handle = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(handle) = self.task_handle.take() {
            handle.stop();
            Ok(())
        } else {
            Err(anyhow!("dns task already stopped or was never started"))
        }
    }
}

impl Drop for CaptivePortalDns {
    fn drop(&mut self) {
        if self.task_handle.is_some() {
            self.stop().unwrap();
        }
    }
}

/// DNS Header Packet
#[repr(C, packed)]
struct DnsHeader {
    id: u16,
    flags: u16,
    qd_count: u16,
    an_count: u16,
    ns_count: u16,
    ar_count: u16,
}

/// DNS Question Packet
#[repr(C)]
struct DnsQuestion {
    typ: u16,
    class: u16,
}

/// DNS Answer Packet
#[repr(C, packed)]
struct DnsAnswer {
    name_ptr: u16,
    typ: u16,
    class: u16,
    ttl: u32,
    addr_len: u16,
    ip_addr: u32,
}

fn parse_dns_name(raw_name: *mut u8, parsed_name: &mut [u8]) -> *mut u8 {
    let mut label = raw_name;
    let parsed_name_max_len = parsed_name.len();
    let mut name_itr = parsed_name.iter_mut();
    let mut name_len: usize = 0;

    loop {
        let sub_name_len = unsafe { *label as c_int };
        // (len + 1) since we are adding  a '.'
        name_len += (sub_name_len + 1) as usize;
        if name_len > parsed_name_max_len {
            return core::ptr::null_mut();
        }

        // Copy the sub name that follows the the label
        for i in 0..sub_name_len {
            let ptr = name_itr.next().unwrap();
            *ptr = unsafe { *label.offset((i + 1) as isize) };
        }
        *name_itr.next().unwrap() = '.' as u8;
        label = unsafe { label.offset((sub_name_len + 1) as isize) };

        if unsafe { *label == 0 } {
            break;
        }
    }

    // Terminate the final string, replacing the last '.'
    parsed_name[name_len - 1] = '\0' as u8;
    // Return pointer to first char after the name
    return unsafe { label.offset(1) };
}

fn parse_dns_request(
    req: &mut [u8],
    req_len: usize,
    dns_reply: &mut [u8],
    dns_reply_max_len: usize,
) -> Option<usize> {
    if req_len > dns_reply_max_len {
        return None;
    }

    // Prepare the reply
    dns_reply.fill(0);
    (&mut dns_reply[0..req_len]).copy_from_slice(&req[0..req_len]);

    let header_len = mem::size_of::<DnsHeader>();
    let (header_bytes, rest) = dns_reply.split_at_mut(header_len);

    // Endianess of NW packet different from chip
    let header = unsafe {
        header_bytes
            .as_mut_ptr()
            .cast::<DnsHeader>()
            .as_mut()
            .unwrap()
    };

    debug!(
        "DNS query with header id: 0x{:X}, flags: 0x{:X}, qd_count: {}",
        ntohs(header.id),
        ntohs(header.flags),
        ntohs(header.qd_count)
    );

    // Not a standard query
    if (header.flags & OPCODE_MASK) != 0 {
        return None;
    }

    // Set question response flag
    header.flags |= QR_FLAG;

    let qd_count = ntohs(header.qd_count);
    header.an_count = htons(qd_count);

    let reply_len = qd_count as usize * mem::size_of::<DnsAnswer>() + req_len;
    if reply_len > dns_reply_max_len {
        return None;
    }

    // Pointer to current answer and question
    let (questions, answers) = rest.split_at_mut(req_len - header_len);
    let cur_qd_ptr = questions.as_mut_ptr();
    let mut cur_ans_ptr = answers.as_mut_ptr();
    let mut name: [u8; 128] = [0; 128];

    // Respond to all questions with the ESP32's IP address
    for i in 0..qd_count {
        debug!("answering question {}", i);
        let name_end_ptr = parse_dns_name(cur_qd_ptr, &mut name);
        if name_end_ptr.is_null() {
            error!("failed to parse DNS question: {:?}", unsafe {
                CStr::from_ptr(cur_qd_ptr as _)
            });
            return None;
        }

        let question = unsafe { name_end_ptr.cast::<DnsQuestion>().as_mut().unwrap() };
        let qd_type = ntohs(question.typ);
        let qd_class = ntohs(question.class);

        info!(
            "received type: {} | class: {} | question for: {:?}",
            qd_type,
            qd_class,
            unsafe { CStr::from_ptr(name.as_ptr() as _) }
        );

        if qd_type == QD_TYPE_A {
            let answer = unsafe { cur_ans_ptr.cast::<DnsAnswer>().as_mut().unwrap() };

            let ptr_offset = unsafe { cur_qd_ptr.offset_from(dns_reply.as_ptr()) };
            answer.name_ptr = htons((0xC000 | ptr_offset) as u16);
            answer.typ = htons(qd_type);
            answer.class = htons(qd_class);
            answer.ttl = htonl(ANS_TTL_SEC);

            let mut ip_info = esp_netif_ip_info_t::default();
            let c_if_key = CString::new("WIFI_AP_DEF").unwrap();
            unsafe {
                esp_netif_get_ip_info(
                    esp_netif_get_handle_from_ifkey(c_if_key.as_ptr()),
                    &mut ip_info,
                )
            };

            info!(
                "answer with PTR offset: 0x{:X} (0x{:X}) and IP 0x{:X}",
                ntohs(answer.name_ptr),
                ptr_offset,
                ip_info.ip.addr,
            );

            answer.addr_len = htons(mem::size_of_val(&ip_info.ip.addr) as u16);
            answer.ip_addr = ip_info.ip.addr;

            cur_ans_ptr = unsafe { cur_ans_ptr.offset(mem::size_of::<DnsAnswer>() as isize) };
        }
    }

    Some(reply_len)
}

fn dns_server_task() -> Result<()> {
    use lwip::*;

    let mut rx_buffer = [0; 128];

    loop {
        let dest_addr = sockaddr_in {
            sin_addr: in_addr {
                s_addr: htonl(IPADDR_ANY),
            },
            sin_family: AF_INET as u8,
            sin_port: htons(DNS_PORT),
            ..Default::default()
        };

        let mut sock = Socket::open(AddressFamily::Ipv4, SocketType::Dgram, Protocol::Ip)
            .context("creating socket")?;
        info!("socket created");

        sock.bind(&dest_addr as *const sockaddr_in as _)
            .context("binding socket")?;
        info!("socket bound, port {}", DNS_PORT);

        loop {
            info!("waiting for data");

            let (len, source_addr) = sock.recv_from(&mut rx_buffer).context("recv_from")?;

            // Null-terminate whatever we received
            rx_buffer[len] = 0;

            let mut reply = [0; DNS_MAX_LEN];
            let reply_len = parse_dns_request(&mut rx_buffer, len, &mut reply, DNS_MAX_LEN);

            info!(
                "received {} bytes from {} | DNS reply with len: {:?}",
                len, source_addr, reply_len
            );

            if let Some(reply_len) = reply_len {
                sock.send_to(&mut reply[0..reply_len], source_addr)
                    .context("send_to")?;
            } else {
                error!("failed to prepare a DNS reply");
            }
        }
    }
}

/// host to network byte order
fn htonl(n: u32) -> u32 {
    n.to_be()
}

/// host to network byte order
fn htons(n: u16) -> u16 {
    n.to_be()
}

/// network to host byte order
fn ntohl(n: u32) -> u32 {
    u32::from_be(n)
}

/// network to host byte order
fn ntohs(n: u16) -> u16 {
    u16::from_be(n)
}
