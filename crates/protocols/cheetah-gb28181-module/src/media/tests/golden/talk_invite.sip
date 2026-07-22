INVITE sip:34020000001320000001@192.168.1.20:5060 SIP/2.0
Via: SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234
From: <sip:server@192.168.1.10:5060>;tag=tag-local
To: <sip:34020000001320000001@192.168.1.20:5060>
Call-ID: call-1
CSeq: 1 INVITE
Contact: <sip:server@192.168.1.10:5060>
Max-Forwards: 70
Subject: 34020000001320000001:0200000000,server:34020000001320000001
Content-Type: application/sdp
Content-Length: 174

v=0
o=- 0 0 IN IP4 192.168.1.100
s=Talk
c=IN IP4 192.168.1.100
t=0 0
m=audio 5000 RTP/AVP 8
c=IN IP4 192.168.1.100
a=sendrecv
a=connection:new
a=rtpmap:8 PCMA/8000
