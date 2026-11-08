// Original ArcaneVR overlay client. Windows/OpenVR SDK only; no desktop-wide capture.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <winhttp.h>
#include <tlhelp32.h>
#include <openvr.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cctype>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

using Clock = std::chrono::steady_clock;
static std::atomic<bool> running{true};
static std::atomic<int> pending{0};
static std::string escape(const std::string& s) {
    std::string out;
    for (unsigned char c : s) {
        if(c=='"'||c=='\\') {out+='\\';out+=c;}
        else if(c=='\n') out+="\\n";
        else if(c=='\r') out+="\\r";
        else if(c=='\t') out+="\\t";
        else if(c>=32) out+=c;
    } return out;
}
static void emit(const char* type,const std::string& message) {
    std::cout << "{\"type\":\"" << type << "\",\"message\":\"" << escape(message) << "\"}" << std::endl;
}
struct Options { bool dashboard=false,test=false; DWORD parent=0; int fps=15; unsigned short port=8811; };
static unsigned short apiPort=8811;
static unsigned long long integer(const std::string& s,unsigned long long max) {
    if(s.empty()||s.find_first_not_of("0123456789")!=std::string::npos) throw std::string("Expected unsigned decimal integer");
    size_t pos=0; unsigned long long n=0;
    try { n=std::stoull(s,&pos); } catch(...) {throw std::string("Integer out of range");}
    if(pos!=s.size()||n>max) throw std::string("Integer out of range"); return n;
}
static Options parse(int argc,char** argv) {
    Options o;
    for(int i=1;i<argc;i++) {
        std::string k=argv[i];
        if(k=="--dashboard") o.dashboard=true;
        else if(k=="--self-test") o.test=true;
        else {
            if(++i>=argc) throw std::string("Missing value for ")+k;
            std::string v=argv[i];
            if(k=="--parent-pid") {o.parent=static_cast<DWORD>(integer(v,MAXDWORD));if(!o.parent) throw std::string("Parent PID must be positive");}
            else if(k=="--http-port") {o.port=static_cast<unsigned short>(integer(v,65535));if(!o.port)throw std::string("HTTP port must be 1 through 65535");}
            else if(k=="--fps") {o.fps=static_cast<int>(integer(v,30));if(o.fps!=15&&o.fps!=30) throw std::string("FPS must be 15 or 30");}
            else throw std::string("Unknown option: ")+k;
        }
    }
    if(o.dashboard==o.test) throw std::string("Choose exactly one: --dashboard or --self-test");
    return o;
}
static std::string commandBody(const std::string& command) {
    if(command!="stop"&&command!="calibrate"&&command!="arm") throw std::string("Command rejected");
    return "{\"command\":\""+command+"\"}";
}
// Extract only top-level fields: nested tracker fields must never be mistaken for state.
static void whitespace(const std::string& s,size_t& i){while(i<s.size()&&(s[i]==' '||s[i]=='\n'||s[i]=='\r'||s[i]=='\t'))i++;}
static bool jsonString(const std::string& s,size_t& i,std::string& out){
    if(i>=s.size()||s[i++]!='"')return false;out.clear();
    while(i<s.size()){char c=s[i++];if(c=='"')return true;if(static_cast<unsigned char>(c)<32)return false;
        if(c=='\\'){if(i>=s.size())return false;char e=s[i++];if(e=='"'||e=='\\'||e=='/')c=e;else if(e=='n'||e=='r'||e=='t')c=' ';else if(e=='u'){if(i+4>s.size())return false;for(int n=0;n<4;n++)if(!std::isxdigit(static_cast<unsigned char>(s[i++])))return false;c='?';}else if(e=='b'||e=='f')c=' ';else return false;}out+=c;}
    return false;
}
static bool skipJson(const std::string& s,size_t& i,unsigned depth=0){
    whitespace(s,i);if(i>=s.size()||depth>64)return false;
    if(s[i]=='"'){std::string ignored;return jsonString(s,i,ignored);}
    if(s[i]=='{'||s[i]=='['){char open=s[i++],close=open=='{'?'}':']';while(i<s.size()){whitespace(s,i);if(i>=s.size())return false;if(s[i]==close){i++;return true;}if(s[i]==','||s[i]==':'){i++;continue;}if(!skipJson(s,i,depth+1))return false;}return false;}
    size_t start=i;while(i<s.size()&&s[i]!=','&&s[i]!='}'&&s[i]!=']'&&!std::isspace(static_cast<unsigned char>(s[i])))i++;return i>start;
}
static bool rootField(const std::string& s,const std::string& wanted,std::string& value){
    size_t i=0;whitespace(s,i);if(i>=s.size()||s[i++]!='{')return false;
    while(i<s.size()){whitespace(s,i);if(i>=s.size()||s[i]=='}')return false;std::string key;if(!jsonString(s,i,key))return false;
        whitespace(s,i);if(i>=s.size()||s[i++]!=':')return false;whitespace(s,i);size_t start=i;if(!skipJson(s,i))return false;
        if(key==wanted){value=s.substr(start,i-start);return true;}whitespace(s,i);if(i>=s.size()||s[i++]!=',')return false;}
    return false;
}
struct BackendState { bool valid=false,calibrated=false,armed=false; };
static bool parseState(const std::string& body,BackendState& state){
    std::string armed,calibrated;if(!rootField(body,"armed",armed)||!rootField(body,"calibrated",calibrated)||(armed!="true"&&armed!="false")||(calibrated!="true"&&calibrated!="false")){state.valid=false;return false;}
    state={true,calibrated=="true",armed=="true"};return true;
}
static bool parseReply(unsigned status,const std::string& body,std::string& error){
    std::string field;if(rootField(body,"error",field)){size_t i=0;std::string detail;if(jsonString(field,i,detail)&&!detail.empty()){error=detail.substr(0,220);return false;}}
    if(status>=200&&status<300&&rootField(body,"ok",field)&&field=="true"){error.clear();return true;}
    error=status>=200&&status<300?"Invalid backend response":"Backend rejected command";return false;
}
// Numeric loopback, no proxy, no redirects, bounded timeout and response size.
static std::string requestError;
static bool request(const std::string& command={},BackendState* state=nullptr) {
    requestError="Backend unavailable";
    std::string body=command.empty()?"":commandBody(command);
    HINTERNET session=WinHttpOpen(L"ArcaneVROverlay/1",WINHTTP_ACCESS_TYPE_NO_PROXY,WINHTTP_NO_PROXY_NAME,WINHTTP_NO_PROXY_BYPASS,0);
    if(!session) return false;
    WinHttpSetTimeouts(session,400,400,400,400);
    HINTERNET connection=WinHttpConnect(session,L"127.0.0.1",apiPort,0);
    HINTERNET req=connection?WinHttpOpenRequest(connection,command.empty()?L"GET":L"POST",command.empty()?L"/api/state":L"/api/command",nullptr,WINHTTP_NO_REFERER,WINHTTP_DEFAULT_ACCEPT_TYPES,0):nullptr;
    bool ok=false;
    if(req) {
        DWORD disabled=WINHTTP_DISABLE_REDIRECTS; WinHttpSetOption(req,WINHTTP_OPTION_DISABLE_FEATURE,&disabled,sizeof(disabled));
        if(WinHttpSendRequest(req,L"Content-Type: application/json\r\n",static_cast<DWORD>(-1L),body.empty()?WINHTTP_NO_REQUEST_DATA:body.data(),static_cast<DWORD>(body.size()),static_cast<DWORD>(body.size()),0)&&WinHttpReceiveResponse(req,nullptr)) {
            DWORD status=0,len=sizeof(status);
            bool headers=WinHttpQueryHeaders(req,WINHTTP_QUERY_STATUS_CODE|WINHTTP_QUERY_FLAG_NUMBER,WINHTTP_HEADER_NAME_BY_INDEX,&status,&len,WINHTTP_NO_HEADER_INDEX)!=FALSE;
            std::string response;char buffer[4096];DWORD count=0;bool readOk=true;
            do {if(!WinHttpReadData(req,buffer,sizeof buffer,&count)){readOk=false;break;}if(response.size()+count>262144){readOk=false;break;}response.append(buffer,count);} while(count);
            if(headers&&readOk){if(command.empty()){ok=status>=200&&status<300&&state&&parseState(response,*state);if(!ok)requestError="Invalid backend state";}else ok=parseReply(status,response,requestError);}
        } WinHttpCloseHandle(req);
    }
    if(connection) WinHttpCloseHandle(connection);WinHttpCloseHandle(session);return ok;
}
struct Surface {
    HDC dc=nullptr;HBITMAP bitmap=nullptr;HGDIOBJ previous=nullptr;void* pixels=nullptr;int width=0,height=0;
    ~Surface(){clear();}
    void clear(){if(dc&&previous)SelectObject(dc,previous);if(bitmap)DeleteObject(bitmap);if(dc)DeleteDC(dc);dc=nullptr;bitmap=nullptr;previous=nullptr;pixels=nullptr;}
    bool resize(int w,int h){if(dc&&w==width&&h==height)return true;clear();width=w;height=h;
        if(w<1||h<1||w>1920||h>1080)return false;
        BITMAPINFO info{};info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER);info.bmiHeader.biWidth=w;info.bmiHeader.biHeight=-h;info.bmiHeader.biPlanes=1;info.bmiHeader.biBitCount=32;info.bmiHeader.biCompression=BI_RGB;
        dc=CreateCompatibleDC(nullptr);bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&pixels,nullptr,0);if(!dc||!bitmap){clear();return false;}previous=SelectObject(dc,bitmap);return true;
    }
    void rgba(){auto p=static_cast<unsigned char*>(pixels);for(size_t i=0;i<static_cast<size_t>(width)*height;i++,p+=4){std::swap(p[0],p[2]);p[3]=255;}}
};
// The SDK may keep reading SetOverlayRaw memory after the call returns.
// No render, resize or second submission may occur until a completion event.
struct UploadGate {
    bool inFlight=false,loaded=false;int failures=0;
    Clock::time_point sent{},retryAt{};
    bool ready(Clock::time_point now,bool visible)const{return !inFlight&&now>=retryAt&&(visible||!loaded);}
    void submitted(Clock::time_point now){inFlight=true;sent=now;}
    void completed(){inFlight=false;loaded=true;failures=0;}
    void failed(Clock::time_point now){inFlight=false;failures++;retryAt=now+std::chrono::milliseconds(std::min(2000,250*(1<<std::min(failures-1,3))));}
    bool timedOut(Clock::time_point now)const{return inFlight&&now-sent>std::chrono::seconds(5);}
};
static void textureStatus(vr::IVROverlay* overlays,vr::VROverlayHandle_t overlay,int expectedW,int expectedH){
    uint32_t w=0,h=0;auto sizeError=overlays->GetOverlayTextureSize(overlay,&w,&h);bool verified=false;int imageError=-1;
    if(sizeError==vr::VROverlayError_None&&w>0&&h>0&&w<=1920&&h<=1080){std::vector<unsigned char> image(static_cast<size_t>(w)*h*4);uint32_t readW=0,readH=0;auto result=overlays->GetOverlayImageData(overlay,image.data(),static_cast<uint32_t>(image.size()),&readW,&readH);imageError=static_cast<int>(result);bool nonzeroRgb=false;for(size_t i=0;i<image.size();i+=4)if(image[i]||image[i+1]||image[i+2]){nonzeroRgb=true;break;}verified=result==vr::VROverlayError_None&&readW==static_cast<uint32_t>(expectedW)&&readH==static_cast<uint32_t>(expectedH)&&nonzeroRgb;}
    std::cout<<"{\"type\":\"texture\",\"width\":"<<w<<",\"height\":"<<h<<",\"size_error\":"<<static_cast<int>(sizeError)<<",\"image_error\":"<<imageError<<",\"readback_verified\":"<<(verified?"true":"false")<<"}"<<std::endl;
}
static void text(HDC dc,const wchar_t* s,int x,int y,int size,COLORREF color){
    HFONT f=CreateFontW(size,0,0,0,FW_BOLD,FALSE,FALSE,FALSE,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,ANTIALIASED_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    HGDIOBJ old=SelectObject(dc,f);SetBkMode(dc,TRANSPARENT);SetTextColor(dc,color);TextOutW(dc,x,y,s,static_cast<int>(wcslen(s)));SelectObject(dc,old);DeleteObject(f);
}
static void rectangle(HDC dc,RECT r,COLORREF c){HBRUSH b=CreateSolidBrush(c);FillRect(dc,&r,b);DeleteObject(b);}
// Button events may omit position. Retain each cursor's last MouseMove,
// as Valve's overlay sample does, then convert GL to top-origin GDI pixels.
struct DashboardInput {
    std::map<uint32_t, std::pair<float,float>> cursors;
    int event(const vr::VREvent_t& e) {
        const auto& m=e.data.mouse;
        if(e.eventType==vr::VREvent_MouseMove) {
            if(std::isfinite(m.x)&&std::isfinite(m.y)) cursors[m.cursorIndex]={m.x,600.f-m.y};
            return 0;
        }
        if(e.eventType!=vr::VREvent_MouseButtonDown || !(m.button&vr::VRMouseButton_Left))return 0;
        auto found=cursors.find(m.cursorIndex);if(found==cursors.end())return 0;
        auto [x,y]=found->second;
        if(x>=850&&x<=975&&y>=20&&y<=85)return 4;
        if(y>=110&&y<=165&&x>=530&&x<=975)return 3;
        if(y<180||y>420)return 0;
        if(x>=50&&x<=340)return 1;
        if(x>=365&&x<=660)return 5;
        return x>=685&&x<=975?2:0;
    }
};
static void dashboard(Surface& s,const std::string& message,bool connected,const BackendState& backend,int seconds){
    rectangle(s.dc,{0,0,1024,600},RGB(17,23,34));text(s.dc,L"ArcaneVR headset controls",55,35,48,RGB(235,242,255));
    rectangle(s.dc,{850,20,975,85},RGB(70,76,88));text(s.dc,L"Close",868,34,29,RGB(255,255,255));
    rectangle(s.dc,{50,180,340,420},RGB(30,93,152));rectangle(s.dc,{365,180,660,420},RGB(29,123,90));rectangle(s.dc,{685,180,975,420},RGB(168,40,51));
    text(s.dc,L"Reset",102,245,42,RGB(255,255,255));text(s.dc,L"Center",100,293,42,RGB(255,255,255));
    text(s.dc,L"START",430,245,44,RGB(255,255,255));text(s.dc,L"MOVEMENT",395,293,35,RGB(255,255,255));text(s.dc,L"STOP",755,265,50,RGB(255,255,255));
    rectangle(s.dc,{530,110,975,165},RGB(42,69,92));text(s.dc,L"Place controls in front of me",545,122,26,RGB(255,255,255));
    std::wstring state=connected&&backend.valid?(backend.calibrated?L"Calibrated":L"Not calibrated"):L"Backend unavailable";
    state+=connected&&backend.valid?(backend.armed?L" | Movement ON":L" | Movement stopped"):L"";
    text(s.dc,state.c_str(),55,110,23,connected?RGB(104,225,167):RGB(255,157,130));
    std::wstring detail=seconds>0?L"Stand still. Reset in "+std::to_wstring(seconds)+L"...":std::wstring(message.begin(),message.end());
    if(detail.size()>75)detail=detail.substr(0,72)+L"...";
    text(s.dc,detail.c_str(),55,460,23,RGB(220,230,244));text(s.dc,L"Reset stops movement. START MOVEMENT is always explicit.",55,535,22,RGB(171,185,206));s.rgba();
}
static bool placeControls(vr::IVRSystem* system,vr::VROverlayHandle_t handle) {
    vr::TrackedDevicePose_t poses[vr::k_unMaxTrackedDeviceCount]{};
    system->GetDeviceToAbsoluteTrackingPose(vr::TrackingUniverseStanding,0,poses,vr::k_unMaxTrackedDeviceCount);
    const auto& p=poses[0];if(!p.bPoseIsValid||!p.bDeviceIsConnected)return false;
    const auto& h=p.mDeviceToAbsoluteTracking;
    for(int r=0;r<3;r++)for(int c=0;c<4;c++)if(!std::isfinite(h.m[r][c]))return false;
    float length=std::hypot(h.m[0][2],h.m[2][2]);if(length<.1f)return false;
    float x=h.m[0][2]/length,z=h.m[2][2]/length;
    vr::HmdMatrix34_t m{{{z,0,x,h.m[0][3]-x*1.5f},{0,1,0,h.m[1][3]},{-x,0,z,h.m[2][3]-z*1.5f}}};
    return vr::VROverlay()->SetOverlayTransformAbsolute(handle,vr::TrackingUniverseStanding,&m)==vr::VROverlayError_None;
}
static bool steamVrRunning(){
    HANDLE snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0);if(snapshot==INVALID_HANDLE_VALUE)return false;
    PROCESSENTRY32W entry{};entry.dwSize=sizeof(entry);bool found=false;
    if(Process32FirstW(snapshot,&entry))do{if(_wcsicmp(entry.szExeFile,L"vrserver.exe")==0){found=true;break;}}while(Process32NextW(snapshot,&entry));
    CloseHandle(snapshot);return found;
}
// The dashboard takes precedence whenever SteamVR's dashboard is open.
static bool showPlaced(bool requested,bool dashboardVisible){return requested&&!dashboardVisible;}
struct DashboardSingleton {
    HANDLE handle=nullptr;
    bool acquire(const wchar_t* name=L"Local\\ArcaneVR.Controls.Singleton") {
        handle=CreateMutexW(nullptr,FALSE,name);
        return handle && GetLastError()!=ERROR_ALREADY_EXISTS;
    }
    ~DashboardSingleton(){if(handle)CloseHandle(handle);}
};
static int selfTest(){
    int checks=0;auto check=[&](bool b){if(!b)throw std::string("Self-test failed at ")+std::to_string(checks);checks++;};
    check(commandBody("stop")=="{\"command\":\"stop\"}");check(commandBody("calibrate").size()<64);check(commandBody("arm")=="{\"command\":\"arm\"}");
    bool unknownRejected=false;try{commandBody("quit");}catch(const std::string&){unknownRejected=true;}check(unknownRejected);
    std::string error;check(parseReply(200,"{\"ok\":true}",error));check(!parseReply(400,"{\"error\":\"Calibrate first\"}",error)&&error=="Calibrate first");
    check(!parseReply(200,"{\"error\":\"Tracker stale\"}",error)&&error=="Tracker stale");check(!parseReply(200,"{}",error));
    BackendState state;check(parseState("{\"nested\":{\"armed\":true},\"armed\":false,\"calibrated\":true}",state)&&state.valid&&!state.armed&&state.calibrated);
    check(!parseState("{\"nested\":{\"armed\":true},\"calibrated\":true}",state)&&!state.valid);
    for(const char* value:{"-1","0x123","18446744073709551616"}){bool rejected=false;try{integer(value,MAXDWORD);}catch(...){rejected=true;}check(rejected);}
    auto rejectArgs=[&](std::initializer_list<const char*> input){std::vector<char*> args;for(auto s:input)args.push_back(const_cast<char*>(s));bool rejected=false;try{parse(static_cast<int>(args.size()),args.data());}catch(...){rejected=true;}check(rejected);};
    rejectArgs({"test","--flat"});rejectArgs({"test","--window","0"});rejectArgs({"test","--dashboard","--flat"});rejectArgs({"test","--dashboard","--fps","14"});rejectArgs({"test","--dashboard","--host","example.com"});rejectArgs({"test","--list-windows"});
    rejectArgs({"test","--dashboard","--http-port","0"});rejectArgs({"test","--dashboard","--http-port","65536"});
    char a0[]="test",a1[]="--dashboard",a2[]="--http-port",a3[]="18815";char* portArgs[]={a0,a1,a2,a3};check(parse(4,portArgs).port==18815);
    UploadGate gate;auto now=Clock::time_point{};check(gate.ready(now,false));gate.submitted(now);check(!gate.ready(now,true));check(!gate.timedOut(now+std::chrono::seconds(4)));check(gate.timedOut(now+std::chrono::seconds(6)));gate.completed();check(!gate.ready(now,false));check(gate.ready(now,true));gate.submitted(now);gate.failed(now);check(!gate.ready(now,true));check(gate.ready(now+std::chrono::milliseconds(250),true));for(int i=0;i<10;i++)gate.failed(now);check(gate.retryAt<=now+std::chrono::seconds(2));
    DashboardInput input;vr::VREvent_t e{};e.eventType=vr::VREvent_MouseButtonDown;e.data.mouse.button=vr::VRMouseButton_Left;check(input.event(e)==0);
    e.eventType=vr::VREvent_MouseMove;e.data.mouse.x=700;e.data.mouse.y=300;e.data.mouse.cursorIndex=1;check(input.event(e)==0);
    e.eventType=vr::VREvent_MouseButtonDown;e.data.mouse.x=0;e.data.mouse.y=0;check(input.event(e)==2);
    e.data.mouse.cursorIndex=2;check(input.event(e)==0);
    e.eventType=vr::VREvent_MouseMove;e.data.mouse.x=100;e.data.mouse.y=200;input.event(e);e.eventType=vr::VREvent_MouseButtonDown;check(input.event(e)==1);
    e.eventType=vr::VREvent_MouseMove;e.data.mouse.y=500;input.event(e);e.eventType=vr::VREvent_MouseButtonDown;check(input.event(e)==0);
    e.eventType=vr::VREvent_MouseMove;e.data.mouse.x=700;e.data.mouse.y=470;input.event(e);e.eventType=vr::VREvent_MouseButtonDown;check(input.event(e)==3);
    e.eventType=vr::VREvent_MouseMove;e.data.mouse.x=900;e.data.mouse.y=550;input.event(e);e.eventType=vr::VREvent_MouseButtonDown;check(input.event(e)==4);
    e.eventType=vr::VREvent_MouseMove;e.data.mouse.x=500;e.data.mouse.y=300;input.event(e);e.eventType=vr::VREvent_MouseButtonDown;check(input.event(e)==5);
    check(showPlaced(true,false));check(!showPlaced(true,true));check(!showPlaced(false,false));
    const auto testMutex=L"Local\\ArcaneVR.Controls.Test."+std::to_wstring(GetCurrentProcessId());
    {DashboardSingleton first,second;check(first.acquire(testMutex.c_str()));check(!second.acquire(testMutex.c_str()));}
    {DashboardSingleton after;check(after.acquire(testMutex.c_str()));}
    emit("status","Static self-tests passed: "+std::to_string(checks));return 0;
}
int main(int argc,char** argv){
    try {
        Options o=parse(argc,argv);apiPort=o.port;if(o.test)return selfTest();
        DashboardSingleton singleton;if(!singleton.acquire())throw std::string("Another ArcaneVR controls helper is already running");
        HANDLE parent=o.parent?OpenProcess(SYNCHRONIZE,FALSE,o.parent):nullptr;
        if(o.parent&&!parent)throw std::string("Parent process unavailable");
        // A live parent watch and piped stdin EOF both terminate the owned helper.
        if(GetFileType(GetStdHandle(STD_INPUT_HANDLE))==FILE_TYPE_PIPE) std::thread([]{char line[128];while(fgets(line,sizeof(line),stdin)){std::string s=line;s.erase(s.find_last_not_of("\r\n")+1);if(s=="quit"){running=false;break;}if(s=="stop")pending=2;else if(s=="reset-center")pending=1;else if(s=="place-controls")pending=3;}running=false;}).detach();
        // Do not start SteamVR as a side effect of this helper.
        if(!steamVrRunning()||!vr::VR_IsHmdPresent()){if(parent)CloseHandle(parent);throw std::string("SteamVR/HMD unavailable; start SteamVR and connect a headset first");}
        vr::EVRInitError error=vr::VRInitError_None;auto system=vr::VR_Init(&error,vr::VRApplication_Overlay);
        if(error!=vr::VRInitError_None||!system){if(parent)CloseHandle(parent);throw std::string("SteamVR unavailable: ")+vr::VR_GetVRInitErrorAsEnglishDescription(error);}
        vr::VROverlayHandle_t overlay=0,thumb=0;auto overlays=vr::VROverlay();
        const std::string key="arcanevr.controls.dashboard";
        auto result=overlays->CreateDashboardOverlay(key.c_str(),"ArcaneVR Reset / Stop",&overlay,&thumb);
        if(result!=vr::VROverlayError_None){vr::VR_Shutdown();if(parent)CloseHandle(parent);throw std::string("Cannot create SteamVR overlay: ")+std::to_string(result);}
        overlays->SetOverlayWidthInMeters(overlay,2.f);
        Surface surface,placedSurface;vr::VROverlayHandle_t placed=0;UploadGate placedUpload;DashboardInput placedInput;bool placedVisible=false,placedShown=false,dismissed=false;std::string placedState;DashboardInput dashboardInput;unsigned clickCount=0;UploadGate upload,thumbnailUpload;std::vector<unsigned char> icon;bool connected=false;BackendState backend;std::string message="Ready";auto deadline=Clock::time_point::max(),nextStatus=Clock::now(),nextTextureStatus=Clock::now();std::string renderedState;
        {surface.resize(1024,600);overlays->SetOverlayInputMethod(overlay,vr::VROverlayInputMethod_Mouse);vr::HmdVector2_t scale{{1024,600}};overlays->SetOverlayMouseScale(overlay,&scale);
            icon.assign(64*64*4,255);for(int y=0;y<64;y++)for(int x=0;x<64;x++){size_t i=(y*64+x)*4;bool mark=y>12&&y<49&&(std::abs(x-(17+(y-12)/2))<4||std::abs(x-(47-(y-12)/2))<4);icon[i]=mark?255:30;icon[i+1]=mark?255:93;icon[i+2]=mark?255:152;}
        }
        emit("status","Dashboard ready: open SteamVR dashboard and select ArcaneVR");
        {
            if(overlays->CreateOverlay((key+".placed").c_str(),"ArcaneVR eye-level controls",&placed)!=vr::VROverlayError_None)throw std::string("Cannot create eye-level controls");
            placedSurface.resize(1024,600);overlays->SetOverlayWidthInMeters(placed,1.2f);
            overlays->SetOverlayInputMethod(placed,vr::VROverlayInputMethod_Mouse);vr::HmdVector2_t scale{{1024,600}};overlays->SetOverlayMouseScale(placed,&scale);
            overlays->SetOverlayFlag(placed,vr::VROverlayFlags_VisibleInDashboard,false);
            overlays->SetOverlayFlag(placed,vr::VROverlayFlags_MakeOverlaysInteractiveIfVisible,true);
        }
        pending=3;
        while(running){
            auto tick=Clock::now();if(parent&&WaitForSingleObject(parent,0)!=WAIT_TIMEOUT)break;
            vr::VREvent_t systemEvent{};while(system->PollNextEvent(&systemEvent,sizeof(systemEvent))){if(systemEvent.eventType==vr::VREvent_Quit)running=false;}
            vr::VREvent_t event{};while(overlays->PollNextOverlayEvent(overlay,&event,sizeof(event))){
                if(event.eventType==vr::VREvent_Quit)running=false;
                if(event.eventType==vr::VREvent_ImageLoaded&&upload.inFlight)upload.completed();
                if(event.eventType==vr::VREvent_ImageFailed&&upload.inFlight){upload.failed(tick);emit("error","SteamVR image load failed; bounded retry scheduled");}
                {int selectedAction=dashboardInput.event(event);if(selectedAction){pending=selectedAction;clickCount++;emit("input",std::string(selectedAction==5?"Start movement":selectedAction==4?"Close":selectedAction==3?"Place controls":selectedAction==2?"STOP":"Reset Center")+" click received #"+std::to_string(clickCount));}}
            }
            if(placed){while(overlays->PollNextOverlayEvent(placed,&event,sizeof(event))){
                if(event.eventType==vr::VREvent_ImageLoaded)placedUpload.completed();
                if(event.eventType==vr::VREvent_ImageFailed)placedUpload.failed(tick);
                int a=placedInput.event(event);if(a){pending=a;clickCount++;}
            }}
            if(thumb){while(overlays->PollNextOverlayEvent(thumb,&event,sizeof(event))){if(event.eventType==vr::VREvent_ImageLoaded&&thumbnailUpload.inFlight)thumbnailUpload.completed();if(event.eventType==vr::VREvent_ImageFailed&&thumbnailUpload.inFlight)thumbnailUpload.failed(tick);}
                if(thumbnailUpload.ready(tick,true)&&!thumbnailUpload.loaded){if(overlays->SetOverlayRaw(thumb,icon.data(),64,64,4)==vr::VROverlayError_None)thumbnailUpload.submitted(tick);else thumbnailUpload.failed(tick);}}
            if(upload.timedOut(tick)||thumbnailUpload.timedOut(tick)){emit("error","SteamVR image completion timed out; closing overlay without reusing upload buffer");break;}
            if(upload.failures>=5||thumbnailUpload.failures>=5){emit("error","SteamVR texture retry limit reached");break;}
            if(!running)break;
            int action=pending.exchange(0);
            if(action==4){dismissed=true;emit("dismissed","Controls closed. Reopen them from ArcaneVR when needed.");break;}
            if(action==3&&placed){bool ok=placeControls(system,placed);if(ok)placedVisible=true;message=ok?"Controls placed at eye level":"Placement failed: wake headset and look forward";emit(ok?"status":"error",message);}
            if(action==5){
                if(deadline!=Clock::time_point::max()){message="Wait for Reset Center to finish before starting";emit("error",message);}
                else {bool accepted=request("arm");message=accepted?"Start accepted - checking movement status":"Start rejected: "+requestError;emit(accepted?"status":"error",message);nextStatus=tick;}
            }
            if(action==2){deadline=Clock::time_point::max();connected=request("stop");message=connected?"STOP confirmed #"+std::to_string(clickCount)+" - movement stopped":"Stop failed: backend unavailable";emit(connected?"status":"error",message);}
            if(action==1){connected=request("stop");if(!connected){deadline=Clock::time_point::max();message="Reset cancelled: backend stop failed";emit("error",message);}else{deadline=Clock::now()+std::chrono::seconds(3);message="Reset countdown started";emit("status",message);}}
            if(running&&Clock::now()>=deadline){deadline=Clock::time_point::max();connected=request("calibrate");message=connected?"Center reset. Movement remains stopped":"Calibration failed: "+requestError;emit(connected?"status":"error",message);}
            if(tick>=nextStatus){if(placed&&!placedVisible&&placeControls(system,placed)){placedVisible=true;emit("status","Controls recovered at eye level");}connected=request({},&backend);if(!connected)backend.valid=false;emit("status",connected?"Backend connected":"Backend unavailable");nextStatus=tick+std::chrono::seconds(1);}
            if(upload.loaded&&tick>=nextTextureStatus){textureStatus(overlays,overlay,surface.width,surface.height);nextTextureStatus=tick+std::chrono::seconds(2);}
            if(placed){
                bool visible=showPlaced(placedVisible,overlays->IsDashboardVisible());
                bool shouldShow=visible&&placedUpload.loaded;
                if(shouldShow!=placedShown){if(shouldShow)overlays->ShowOverlay(placed);else overlays->HideOverlay(placed);placedShown=shouldShow;}
                int seconds=deadline==Clock::time_point::max()?0:std::max(1,static_cast<int>(std::ceil(std::chrono::duration<double>(deadline-Clock::now()).count())));
                std::string state=message+"|"+std::to_string(connected)+"|"+std::to_string(backend.valid)+"|"+std::to_string(backend.calibrated)+"|"+std::to_string(backend.armed)+"|"+std::to_string(seconds);
                if(placedUpload.ready(tick,visible)&&(!placedUpload.loaded||placedState!=state)){
                    dashboard(placedSurface,message,connected,backend,seconds);
                    if(overlays->SetOverlayRaw(placed,placedSurface.pixels,1024,600,4)==vr::VROverlayError_None){placedUpload.submitted(tick);placedState=state;}else placedUpload.failed(tick);
                }
                if(placedUpload.timedOut(tick)||placedUpload.failures>=5){emit("error","Eye-level panel upload failed");break;}
            }
            if(!upload.ready(tick,overlays->IsOverlayVisible(overlay))){std::this_thread::sleep_until(tick+std::chrono::milliseconds(1000/o.fps));continue;}
            if(!surface.dc){emit("error","Dashboard texture allocation failed");break;}int seconds=deadline==Clock::time_point::max()?0:std::max(1,static_cast<int>(std::ceil(std::chrono::duration<double>(deadline-Clock::now()).count())));std::string stateKey=message+"|"+std::to_string(connected)+"|"+std::to_string(backend.valid)+"|"+std::to_string(backend.calibrated)+"|"+std::to_string(backend.armed)+"|"+std::to_string(seconds);
                if(upload.loaded&&stateKey==renderedState){std::this_thread::sleep_until(tick+std::chrono::milliseconds(1000/o.fps));continue;}
                dashboard(surface,message,connected,backend,seconds);renderedState=stateKey;
            if(overlays->SetOverlayRaw(overlay,surface.pixels,surface.width,surface.height,4)==vr::VROverlayError_None)upload.submitted(Clock::now());else{upload.failed(Clock::now());emit("error","SteamVR rejected overlay texture; bounded retry scheduled");}
            std::this_thread::sleep_until(tick+std::chrono::milliseconds(1000/o.fps));
        }
        if(placed){overlays->HideOverlay(placed);overlays->DestroyOverlay(placed);}
        overlays->HideOverlay(overlay);if(thumb)overlays->DestroyOverlay(thumb);overlays->DestroyOverlay(overlay);vr::VR_Shutdown();if(parent)CloseHandle(parent);emit("status","Overlay closed");return dismissed?42:0;
    }catch(const std::string& e){emit("error",e);return 2;}catch(const std::exception& e){emit("error",e.what());return 2;}
}
