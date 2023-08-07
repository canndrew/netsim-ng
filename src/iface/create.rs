use crate::priv_prelude::*;

pub struct IpPacketStream {
    fd: Arc<AsyncFd<OwnedFd>>,
}

pub struct IpPacketSink {
    fd: Arc<AsyncFd<OwnedFd>>,
    packet_opt: Option<Vec<u8>>,
}

struct BuildConfig {
    name_opt: Option<String>,
    ipv4_addr_subnet_opt: Option<(Ipv4Addr, u8)>,
}

pub struct IpIfaceBuilder<'m> {
    machine: &'m Machine,
    build_config: BuildConfig,
}

impl IpIfaceBuilder<'_> {
    pub(crate) fn new(machine: &Machine) -> IpIfaceBuilder<'_> {
        IpIfaceBuilder {
            machine,
            build_config: BuildConfig {
                name_opt: None,
                ipv4_addr_subnet_opt: None,
            },
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.build_config.name_opt = Some(name.into());
        self
    }

    pub fn ipv4_addr(mut self, ipv4_addr: impl Into<Ipv4Addr>) -> Self {
        let ipv4_addr = ipv4_addr.into();
        let network = Ipv4Network::infer_from_addr(ipv4_addr);
        self.build_config.ipv4_addr_subnet_opt = Some((ipv4_addr, network.subnet_mask_bits()));
        self
    }
}

impl<'m> IntoFuture for IpIfaceBuilder<'m> {
    type Output = io::Result<(IpPacketSink, IpPacketStream)>;
    type IntoFuture = Pin<Box<dyn Future<Output = io::Result<(IpPacketSink, IpPacketStream)>> + Send + 'm>>;
    //type IntoFuture = impl Future<Output = io::Result<(IpPacketSink, IpPacketStream)>> + Send + 'm;

    fn into_future(self) -> Pin<Box<dyn Future<Output = io::Result<(IpPacketSink, IpPacketStream)>> + Send + 'm>> {
        let IpIfaceBuilder { machine, build_config } = self;
        Box::pin(async move {
            let task = async move {
                create_tun_interface(build_config)
            };
            let join_handle = machine.spawn(task).await;
            let res = join_handle.join().await;
            let fd = match res {
                Ok(res_opt) => res_opt.unwrap()?,
                Err(err) => panic::resume_unwind(err),
            };
            let fd = Arc::new(AsyncFd::new(fd)?);
            let ip_packet_sink = IpPacketSink { fd: fd.clone(), packet_opt: None };
            let ip_packet_stream = IpPacketStream { fd };
            Ok((ip_packet_sink, ip_packet_stream))
        })
    }
}

fn create_tun_interface(build_config: BuildConfig) -> io::Result<OwnedFd> {
    let BuildConfig { name_opt, ipv4_addr_subnet_opt } = build_config;
    let name = name_opt.as_deref().unwrap_or("netsim");
    let name_cstr = match CString::new(name) {
        Ok(name_cstr) => name_cstr,
        Err(err) => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, err));
        },
    };
    if name_cstr.as_bytes_with_nul().len() > libc::IF_NAMESIZE as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
    }

    let fd = {
        let raw_fd = unsafe {
            libc::open(b"/dev/net/tun\0".as_ptr() as *const _, libc::O_RDWR)
        };
        if raw_fd < 0 {
            let err = io::Error::last_os_error();
            return Err(io::Error::new(err.kind(), "opening /dev/net/tun"));
        }
        unsafe {
            OwnedFd::from_raw_fd(raw_fd)
        }
    };
    let flags = unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_GETFL, 0)
    };
    if flags < 0 {
        let err = io::Error::last_os_error();
        return Err(io::Error::new(err.kind(), "calling fcntl(F_GETFL) on /dev/net/tun"));
    }
    let res = unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
    };
    if res < 0 {
        let err = io::Error::last_os_error();
        return Err(io::Error::new(err.kind(), "calling fcntl(F_SETFL) on /dev/net/tun"));
    }
    let mut req = unsafe {
        let mut req: libc::ifreq = mem::zeroed();
        ptr::copy_nonoverlapping(
            name_cstr.as_ptr(),
            req.ifr_name.as_mut_ptr(),
            name_cstr.as_bytes().len(),
        );
        req.ifr_ifru.ifru_flags = libc::IFF_NO_PI as i16;
        req.ifr_ifru.ifru_flags |= libc::IFF_TUN as i16;
        req
    };
    let res = unsafe {
        ioctl::tunsetiff(fd.as_raw_fd(), &mut req as *mut _ as *mut _)
    };
    if res < 0 {
        let err = io::Error::last_os_error();
        return Err(io::Error::new(err.kind(), "calling ioctl(TUNSETIFF) failed"));
    };
    let real_name = {
        let name = &req.ifr_name[..];
        let name = match name.iter().position(|b| *b == 0) {
            Some(p) => &name[..p],
            None => name,
        };
        let name = unsafe {
            slice::from_raw_parts(name.as_ptr() as *const _, name.len())
        };
        let name = match std::str::from_utf8(name) {
            Ok(name) => name,
            Err(err) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, err));
            },
        };
        name.to_owned()
    };


    /*
    if let Some(mac_addr) = mac_addr {
        match iface::set_mac_addr(&real_name, mac_addr) {
            Ok(()) => (),
            Err(SetMacAddrError::UnknownInterface)
                => panic!("the interface we just created doesn't exist?"),
            Err(SetMacAddrError::PermissionDenied(..))
                => panic!("don't have permission to configure the interface we just created?"),
            Err(SetMacAddrError::AddrNotAvailable(e))
                => return Err(IfaceBuildError::MacAddrNotAvailable(e)),
            Err(SetMacAddrError::ProcessFileDescriptorLimit(e))
                => return Err(IfaceBuildError::ProcessFileDescriptorLimit(e)),
            Err(SetMacAddrError::SystemFileDescriptorLimit(e))
                => return Err(IfaceBuildError::SystemFileDescriptorLimit(e)),
        }
    }
    */

    if let Some((ipv4_addr, subnet_mask_bits)) = ipv4_addr_subnet_opt {
        iface::configure::set_ipv4_addr(&real_name, ipv4_addr, subnet_mask_bits)?;
    }

    /*
    if let Some((ipv6_addr, ipv6_netmask_bits)) = builder.ipv6_addr {
        match iface::set_ipv6_addr(&real_name, ipv6_addr, ipv6_netmask_bits) {
            Ok(()) => (),
            Err(SetIpv6AddrError::UnknownInterface)
                => panic!("the interface we just created doesn't exist?"),
            Err(SetIpv6AddrError::PermissionDenied(..))
                => panic!("don't have permission to configure the interface we just created?"),
            Err(SetIpv6AddrError::AddrNotAvailable(e))
                => return Err(IfaceBuildError::Ipv6AddrNotAvailable(e)),
            Err(SetIpv6AddrError::ProcessFileDescriptorLimit(e))
                => return Err(IfaceBuildError::ProcessFileDescriptorLimit(e)),
            Err(SetIpv6AddrError::SystemFileDescriptorLimit(e))
                => return Err(IfaceBuildError::SystemFileDescriptorLimit(e)),
        }
    }
    */

    iface::configure::put_up(&real_name)?;

    /*
    for route in builder.ipv4_routes {
        trace!("adding route {:?} to {}", route, real_name);
        match route.add_to_routing_table(&real_name) {
            Ok(()) => (),
            Err(AddRouteError::ProcessFileDescriptorLimit(e)) => {
                return Err(IfaceBuildError::ProcessFileDescriptorLimit(e));
            },
            Err(AddRouteError::SystemFileDescriptorLimit(e)) => {
                return Err(IfaceBuildError::SystemFileDescriptorLimit(e));
            },
            Err(AddRouteError::NameContainsNul) => unreachable!(),
        }
    }

    for route in builder.ipv6_routes {
        trace!("adding route {:?} to {}", route, real_name);
        match route.add_to_routing_table(&real_name) {
            Ok(()) => (),
            Err(AddRouteError::ProcessFileDescriptorLimit(e)) => {
                return Err(IfaceBuildError::ProcessFileDescriptorLimit(e));
            },
            Err(AddRouteError::SystemFileDescriptorLimit(e)) => {
                return Err(IfaceBuildError::SystemFileDescriptorLimit(e));
            },
            Err(AddRouteError::NameContainsNul) => unreachable!(),
        }
    }
    */

    Ok(fd)
}

impl Sink<Vec<u8>> for IpPacketSink {
    type Error = io::Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        Self::poll_flush(self, cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> io::Result<()> {
        let this = self.get_mut();
        let packet_opt = this.packet_opt.replace(item);
        assert!(packet_opt.is_none());
        Ok(())
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let packet = match this.packet_opt.take() {
            Some(packet) => packet,
            None => return Poll::Ready(Ok(())),
        };
        if packet.len() > 1500 {
            return Poll::Ready(Ok(()));
        }
        loop {
            let mut guard = ready!(this.fd.poll_write_ready(cx))?;
            match guard.try_io(|fd| {
                let res = unsafe {
                    libc::write(
                        fd.as_raw_fd(),
                        packet.as_slice().as_ptr() as *const libc::c_void,
                        packet.len(),
                    )
                };
                if res < 0 {
                    let err = io::Error::last_os_error();
                    return Err(err);
                }
                Ok(res as usize)
            }) {
                Ok(Ok(n)) => {
                    assert_eq!(n, packet.len());
                    return Poll::Ready(Ok(()));
                },
                Ok(Err(err)) => return Poll::Ready(Err(err)),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        Self::poll_flush(self, cx)
    }
}

impl Stream for IpPacketStream {
    type Item = io::Result<Vec<u8>>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Option<io::Result<Vec<u8>>>> {
        let this = self.get_mut();
        loop {
            let mut guard = ready!(this.fd.poll_read_ready(cx))?;
            // TODO: don't initialize the buffer once MaybeUninit features are stable
            let mut buffer = [0u8; libc::ETH_FRAME_LEN as usize];
            match guard.try_io(|fd| {
                let res = unsafe {
                    libc::read(
                        fd.as_raw_fd(),
                        buffer.as_mut_slice().as_mut_ptr() as *mut libc::c_void,
                        buffer.len(),
                    )
                };
                if res < 0 {
                    let err = io::Error::last_os_error();
                    return Err(err);
                }
                Ok(res as usize)
            }) {
                Ok(Ok(n)) => {
                    if n == 0 {
                        return Poll::Ready(None);
                    } else {
                        return Poll::Ready(Some(Ok(buffer[..n].to_vec())));
                    }
                },
                Ok(Err(err)) => return Poll::Ready(Some(Err(err))),
                Err(_would_block) => continue,
            }
        }
    }
}
