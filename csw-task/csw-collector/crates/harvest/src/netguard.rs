//! 出网防护：**封内网、回环、链路本地、云元数据地址**（SSRF）。
//!
//! 下载的图片地址与补读的外链都来自第三方——贴文正文、平台返回的媒体地址。
//! 不设防的话，一条正文里写 `http://169.254.169.254/latest/meta-data/` 或
//! `http://127.0.0.1:8022/`，服务就会替它去读云主机凭据、打本机的向量服务。
//!
//! # 三道闸，缺一道都有洞
//!
//! 1. **解析器**（[`GuardResolver`]）：域名解析的结果里剔掉禁用地址，剔空就失败。
//!    连接用的就是它给出的地址，**没有第二次解析**——DNS 重绑定（第一次解析给公网、
//!    第二次给内网）无从下手。
//! 2. **IP 字面量**（[`check_url`]）：`http://127.0.0.1/`、`http://2130706433/`、
//!    `http://[::ffff:127.0.0.1]/` 这类地址不走解析器，要在发请求前单独查。
//!    `url` crate 按 WHATWG 规范把十进制、八进制、十六进制的 IPv4 写法都规整成标准形式，
//!    所以这里拿到的就是真地址。
//! 3. **每一跳重定向**（[`guarded_client`] 的 redirect 策略）：公网地址 302 到内网地址
//!    是最常见的绕法，每一跳都重新过 1、2 两道。
//!
//! 另外关掉系统代理：走代理时解析在代理那边做，第 1 道闸就形同虚设。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

/// 最多跟几跳重定向。
pub const MAX_REDIRECTS: usize = 5;

/// 这个地址能不能连。**默认拒绝一切非公网单播地址。**
pub fn is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => forbidden_v6(v6),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()        // 0.0.0.0
        || o[0] == 0           // 0/8「本网络」
        || ip.is_private()     // 10/8、172.16/12、192.168/16
        || ip.is_loopback()    // 127/8
        || ip.is_link_local()  // 169.254/16，含云元数据 169.254.169.254
        || (o[0] == 100 && (o[1] & 0xC0) == 64) // 100.64/10 运营商级 NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0/24 协议分配
        || (o[0] == 192 && o[1] == 0 && o[2] == 2) // 文档用
        || (o[0] == 198 && (o[1] & 0xFE) == 18)    // 198.18/15 基准测试
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
        || ip.is_multicast()   // 224/4
        || o[0] >= 240 // 240/4 保留，含 255.255.255.255
}

fn forbidden_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // 内嵌 IPv4 的几种写法：按内嵌的那个 IPv4 判
    if let Some(v4) = ip.to_ipv4_mapped() {
        return forbidden_v4(v4); // ::ffff:a.b.c.d
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        // 64:ff9b::/96 NAT64
        return forbidden_v4(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            s[6] as u8,
            (s[7] >> 8) as u8,
            s[7] as u8,
        ));
    }
    if s[..6] == [0, 0, 0, 0, 0, 0] && !(s[6] == 0 && s[7] <= 1) {
        // ::a.b.c.d（已废弃的 IPv4 兼容地址）
        return forbidden_v4(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            s[6] as u8,
            (s[7] >> 8) as u8,
            s[7] as u8,
        ));
    }
    ip.is_unspecified()                 // ::
        || ip.is_loopback()             // ::1
        || (s[0] & 0xFE00) == 0xFC00    // fc00::/7 唯一本地
        || (s[0] & 0xFFC0) == 0xFE80    // fe80::/10 链路本地
        || (s[0] & 0xFF00) == 0xFF00    // ff00::/8 组播
        || (s[0] == 0x2001 && s[1] == 0x0db8) // 文档用
        || (s[0] == 0x0100 && s[1..4] == [0, 0, 0]) // 100::/64 丢弃
}

/// 发请求前（以及每一跳重定向）检查地址本身：协议、端口、IP 字面量、`localhost`。
///
/// `allow_private` 只给测试用（wiremock 监听在 127.0.0.1 的随机端口上）。
pub fn check_url(url: &Url, allow_private: bool) -> Result<(), String> {
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("不许的协议 {other}")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("地址里带了用户名或口令".into());
    }
    if allow_private {
        return Ok(());
    }
    if let Some(p) = url.port()
        && p != 80
        && p != 443
    {
        return Err(format!("不许的端口 {p}"));
    }
    match url.host() {
        None => Err("地址没有主机".into()),
        Some(Host::Ipv4(ip)) if forbidden_v4(ip) => Err(format!("禁止访问的地址 {ip}")),
        Some(Host::Ipv6(ip)) if forbidden_v6(ip) => Err(format!("禁止访问的地址 {ip}")),
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if d == "localhost" || d.ends_with(".localhost") || d.ends_with(".internal") {
                Err(format!("禁止访问的主机 {d}"))
            } else {
                Ok(())
            }
        }
        Some(_) => Ok(()),
    }
}

/// 解析后剔掉禁用地址的解析器。见模块文档第 1 道闸。
#[derive(Debug, Clone, Default)]
pub struct GuardResolver {
    allow_private: bool,
}

impl Resolve for GuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let allow = self.allow_private;
        Box::pin(async move {
            let all: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let ok: Vec<SocketAddr> = all
                .into_iter()
                .filter(|a| allow || !is_forbidden(a.ip()))
                .collect();
            if ok.is_empty() {
                return Err(format!("{host} 解析到的地址都在禁止范围内").into());
            }
            Ok(Box::new(ok.into_iter()) as Addrs)
        })
    }
}

/// 带三道闸的 HTTP 客户端。**所有从第三方拿到的地址都要用它去取。**
pub fn guarded_client(timeout: Duration, allow_private: bool) -> reqwest::Result<reqwest::Client> {
    csw_collector_core::ensure_crypto_provider();
    reqwest::Client::builder()
        .timeout(timeout)
        .no_proxy()
        .dns_resolver(Arc::new(GuardResolver { allow_private }))
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            if a.previous().len() >= MAX_REDIRECTS {
                return a.error(format!("重定向超过 {MAX_REDIRECTS} 跳"));
            }
            match check_url(a.url(), allow_private) {
                Ok(()) => a.follow(),
                Err(e) => a.error(format!("重定向到了{e}")),
            }
        }))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad(u: &str) -> bool {
        check_url(&Url::parse(u).unwrap(), false).is_err()
    }

    #[test]
    fn 内网回环与元数据地址一律拒绝() {
        for u in [
            "http://127.0.0.1/",
            "http://127.1.2.3:80/",
            "http://10.0.0.8/",
            "http://172.16.5.4/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.100.100.200/latest/meta-data/", // 阿里云元数据
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[::]/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[64:ff9b::a9fe:a9fe]/", // NAT64 包着 169.254.169.254
            "http://localhost/",
            "http://api.localhost/",
            "http://metadata.google.internal/",
        ] {
            assert!(bad(u), "{u} 该被拒");
        }
    }

    #[test]
    fn 十进制八进制十六进制的回环写法都认得出() {
        // url crate 按 WHATWG 规整：这些写的都是 127.0.0.1
        for u in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0177.0.0.1/",
            "http://127.1/",
        ] {
            assert!(bad(u), "{u} 该被拒");
        }
    }

    #[test]
    fn 本机服务端口与怪协议都拒绝() {
        assert!(bad("https://example.com:8022/v1/embeddings"));
        assert!(bad("http://example.com:8090/"));
        assert!(bad("file:///etc/passwd"));
        assert!(bad("ftp://example.com/"));
        assert!(bad("gopher://example.com/"));
        assert!(bad("http://user:pw@example.com/"));
    }

    #[test]
    fn 正常的公网地址放行() {
        for u in [
            "https://www.hyperlitemountaingear.com/blogs/news/tents",
            "http://example.com/",
            "https://8.8.8.8/",
            "https://[2606:4700:4700::1111]/",
            "https://example.com:443/x",
        ] {
            assert!(!bad(u), "{u} 该放行");
        }
    }

    #[tokio::test]
    async fn 解析到内网的域名连不上() {
        // localhost 解析出来只有回环地址：剔完就空了
        let r = GuardResolver::default();
        let name: Name = "localhost".parse().unwrap();
        assert!(r.resolve(name).await.is_err());
        // 测试放行开关打开时能解析
        let r = GuardResolver {
            allow_private: true,
        };
        assert!(r.resolve("localhost".parse().unwrap()).await.is_ok());
    }

    #[tokio::test]
    async fn 公网地址重定向到内网被拦下() {
        // 起一个 wiremock 当「公网」站点（测试里只能放它在回环上），
        // 让它 302 到元数据地址：客户端开着放行开关连得上 wiremock，
        // 但重定向那一跳照样要过 check_url——这里用关掉放行的检查函数单独验证那一跳
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(302)
                    .insert_header("Location", "http://169.254.169.254/latest/meta-data/"),
            )
            .mount(&server)
            .await;
        // 放行开关只放行回环的起点；目标地址不许放行——用一个只放行起点的策略
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::custom(|a| {
                match check_url(a.url(), false) {
                    Ok(()) => a.follow(),
                    Err(e) => a.error(e),
                }
            }))
            .build()
            .unwrap();
        let err = client.get(server.uri()).send().await.unwrap_err();
        assert!(err.is_redirect(), "{err:?}");
    }

    #[tokio::test]
    async fn 默认客户端连不上本机() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("内网数据"))
            .mount(&server)
            .await;
        let c = guarded_client(Duration::from_secs(5), false).unwrap();
        // IP 字面量不走解析器：靠调用方先过 check_url。这里模拟调用方漏查——
        // 用域名 localhost 指向同一端口，解析器那一道闸要挡住
        let port = server.address().port();
        let r = c.get(format!("http://localhost:{port}/")).send().await;
        assert!(r.is_err(), "解析器没挡住回环");
    }
}
