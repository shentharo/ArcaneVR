#include <cmath>
#include <cstdint>
#include <cstring>
#include <atomic>
#include <thread>
#include <mutex>
#include <chrono>
#include <cstdio>

// Wire format is packed little endian (Windows x64). Angles are radians.
#pragma pack(push,1)
struct Ack {char magic[4];uint32_t sequence,generation,flags;};
struct AckV2 {char magic[4];uint32_t sequence,generation,flags;float height,yaw,pitch,roll;};
struct Message {char magic[4];uint32_t sequence,generation;float speed,yaw;};
struct MessageV2 {char magic[4];uint32_t sequence,generation;float travelX,travelY,turnX,turnY;uint32_t buttons,flags;float chestYaw;};
#pragma pack(pop)
static_assert(sizeof(Message)==20 && sizeof(MessageV2)==40 && sizeof(Ack)==16 && sizeof(AckV2)==32);

namespace protocol {
class PeriodicWorker {
 std::atomic<bool> running{false};std::thread thread;
public:
 template<class F> void start(F task){running=true;thread=std::thread([this,task](){while(running.load(std::memory_order_acquire)){task();std::this_thread::sleep_for(std::chrono::milliseconds(5));}});}
 void stop(){running.store(false,std::memory_order_release);if(thread.joinable())thread.join();}
 ~PeriodicWorker(){stop();}
};
struct Input {
 uint32_t sequence=0,generation=0,buttons=0,flags=0;
 float travelX=0,travelY=0,turnX=0,turnY=0,chestYaw=0;
 bool extended=false;
 bool neutral()const{return travelX==0 && travelY==0 && turnX==0 && turnY==0 && buttons==0;}
};
static bool axis(float value){return std::isfinite(value) && value>=-1 && value<=1;}
static bool decode(const char* bytes,int size,Input& out) {
 Input next;
 if(size==sizeof(Message) && !std::memcmp(bytes,"ALOC",4)) {
  Message m;std::memcpy(&m,bytes,sizeof m);
  if(!std::isfinite(m.speed) || m.speed<0 || m.speed>1 || !std::isfinite(m.yaw))return false;
  next.sequence=m.sequence;next.generation=m.generation;next.travelY=m.speed;next.chestYaw=m.yaw;
 }else if(size==sizeof(MessageV2) && !std::memcmp(bytes,"ALC2",4)) {
  MessageV2 m;std::memcpy(&m,bytes,sizeof m);
  if(!axis(m.travelX) || !axis(m.travelY) || std::hypot(m.travelX,m.travelY)>1.00001f ||
     !axis(m.turnX) || !axis(m.turnY) || (m.buttons&~7u) || (m.flags&~1u) ||
     !std::isfinite(m.chestYaw) || std::abs(m.chestYaw)>1000)return false;
  next.sequence=m.sequence;next.generation=m.generation;next.travelX=m.travelX;next.travelY=m.travelY;
  next.turnX=m.turnX;next.turnY=m.turnY;next.buttons=m.buttons;next.flags=m.flags;next.chestYaw=m.chestYaw;next.extended=true;
 }else return false;
 out=next;return true;
}
// Expiry latches until a fresh generation starts with every output released.
struct Session {
 Input input{};bool initialized=false,active=false;double last=0;uint16_t port=0;
 bool expire(double now){if(active && now-last>=.25){active=false;return true;}return false;}
 bool accept(const Input& next,double now,uint16_t sourcePort) {
  expire(now);
  if(initialized && sourcePort==port) {
   if(static_cast<int32_t>(next.sequence-input.sequence)<=0)return false;
   if(next.generation==input.generation) {if(!active)return false;}
   else if(static_cast<int32_t>(next.generation-input.generation)<=0 || !next.neutral())return false;
  }else if(initialized) {
   // A new sender can start only after the old session is inactive, with neutral input.
   if(active || !next.neutral())return false;
  }else if(!next.neutral()) {
   return false;
  }
  input=next;last=now;port=sourcePort;initialized=active=true;return true;
 }
};
struct Transport {
 Session session;
 bool expire_before_output(double now){return session.expire(now);}
 // Receive returns bytes and sender port, and on_accept records the accepted address.
 template<class Receive,class Accepted> bool pump(double now,Receive receive,Accepted on_accept) {
  bool reset=session.expire(now);char bytes[128];
  for(int i=0;i<64;i++) {
   uint16_t port=0;int size=receive(bytes,sizeof bytes,port);if(size<=0)break;
   Input next;if(!decode(bytes,size,next))continue;
   bool generation=!session.initialized || next.generation!=session.input.generation;
   if(!session.accept(next,now,port))continue;
   if(generation)reset=true;on_accept();
  }
  return reset;
 }
};
static void travel(const Input& in,float relative,float& x,float& y) {
 if(in.flags&1u){x=in.travelX;y=in.travelY;return;}
 x=std::cos(relative)*in.travelX+std::sin(relative)*in.travelY;
 y=-std::sin(relative)*in.travelX+std::cos(relative)*in.travelY;
}
}

#ifndef ARCANE_PROTOCOL_TEST
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <openvr_driver.h>
#include <chrono>
using namespace vr;
using Clock=std::chrono::steady_clock;

class Treadmill final:public ITrackedDeviceServerDriver {
public:
 std::mutex mutex;
 float appliedX=0,appliedY=0,appliedTurnX=0,appliedTurnY=0;uint32_t appliedButtons=0;uint64_t updates=0;bool updateOk=false;
 TrackedDeviceIndex_t index=k_unTrackedDeviceIndexInvalid;
 VRInputComponentHandle_t x=0,y=0,touch=0,click=0,turnX=0,turnY=0,turnTouch=0,turnClick=0,jump=0,crouch=0,sprint=0;
 DriverPose_t pose{};
 EVRInitError Activate(uint32_t id)override {
  std::lock_guard<std::mutex> lock(mutex);
  index=k_unTrackedDeviceIndexInvalid;updateOk=false;appliedX=appliedY=appliedTurnX=appliedTurnY=0;appliedButtons=0;
  auto props=VRProperties()->TrackedDeviceToPropertyContainer(id);
  VRProperties()->SetStringProperty(props,Prop_ModelNumber_String,"ArcaneVR Locomotion");
  VRProperties()->SetStringProperty(props,Prop_ManufacturerName_String,"ArcaneVR");
  VRProperties()->SetStringProperty(props,Prop_TrackingSystemName_String,"ArcaneVR");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceOff_String,"{arcane_locomotion}/icons/status_off.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceSearching_String,"{arcane_locomotion}/icons/status_searching.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceSearchingAlert_String,"{arcane_locomotion}/icons/status_alert.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceReady_String,"{arcane_locomotion}/icons/status_ready.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceReadyAlert_String,"{arcane_locomotion}/icons/status_alert.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceNotReady_String,"{arcane_locomotion}/icons/status_alert.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceStandby_String,"{arcane_locomotion}/icons/status_off.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceAlertLow_String,"{arcane_locomotion}/icons/status_alert.png");
  VRProperties()->SetStringProperty(props,Prop_NamedIconPathDeviceStandbyAlert_String,"{arcane_locomotion}/icons/status_alert.png");
  VRProperties()->SetStringProperty(props,Prop_SerialNumber_String,"arcane-locomotion-001");
  VRProperties()->SetStringProperty(props,Prop_ControllerType_String,"arcane_locomotion");
  VRProperties()->SetInt32Property(props,Prop_ControllerRoleHint_Int32,TrackedControllerRole_Treadmill);
  VRProperties()->SetStringProperty(props,Prop_InputProfilePath_String,"{arcane_locomotion}/input/treadmill_profile.json");
  VRProperties()->SetBoolProperty(props,Prop_DeviceProvidesBatteryStatus_Bool,false);
  VRProperties()->SetBoolProperty(props,Prop_NeverTracked_Bool,true);
  auto scalar=[&](const char* name,VRInputComponentHandle_t& handle){return VRDriverInput()->CreateScalarComponent(props,name,&handle,VRScalarType_Absolute,VRScalarUnits_NormalizedTwoSided)==VRInputError_None;};
  auto boolean=[&](const char* name,VRInputComponentHandle_t& handle){return VRDriverInput()->CreateBooleanComponent(props,name,&handle)==VRInputError_None;};
  if(!scalar("/input/joystick/x",x) || !scalar("/input/joystick/y",y) ||
     !scalar("/input/turn/x",turnX) || !scalar("/input/turn/y",turnY) ||
     !boolean("/input/joystick/touch",touch) || !boolean("/input/joystick/click",click) ||
     !boolean("/input/turn/touch",turnTouch) || !boolean("/input/turn/click",turnClick) ||
     !boolean("/input/jump/click",jump) || !boolean("/input/crouch/click",crouch) || !boolean("/input/sprint/click",sprint)) {
   x=y=touch=click=turnX=turnY=turnTouch=turnClick=jump=crouch=sprint=0;
   return VRInitError_Driver_Failed;
  }
  pose.qWorldFromDriverRotation.w=pose.qDriverFromHeadRotation.w=pose.qRotation.w=1;
  pose.deviceIsConnected=true;pose.poseIsValid=true;pose.result=TrackingResult_Running_OK;
  index=id;
  return VRInitError_None;
 }
 void Deactivate()override{std::lock_guard<std::mutex> lock(mutex);index=k_unTrackedDeviceIndexInvalid;appliedX=appliedY=appliedTurnX=appliedTurnY=0;appliedButtons=0;updateOk=false;}
 void EnterStandby()override{}
 void*GetComponent(const char*)override{return nullptr;}
 void DebugRequest(const char* request,char*out,uint32_t n)override{
  if(!out || !n)return;std::lock_guard<std::mutex> lock(mutex);
  if(!request || std::strcmp(request,"arcane-status")){out[0]=0;return;}
  std::snprintf(out,n,"{\"active\":%s,\"update_ok\":%s,\"x\":%.6f,\"y\":%.6f,\"turn_x\":%.6f,\"turn_y\":%.6f,\"buttons\":%u,\"updates\":%llu}",index==k_unTrackedDeviceIndexInvalid?"false":"true",updateOk?"true":"false",appliedX,appliedY,appliedTurnX,appliedTurnY,appliedButtons,static_cast<unsigned long long>(updates));
 }
 DriverPose_t GetPose()override{std::lock_guard<std::mutex> lock(mutex);return pose;}
 void update(float vx=0,float vy=0,float tx=0,float ty=0,uint32_t buttons=0) {
  std::lock_guard<std::mutex> lock(mutex);
  if(index==k_unTrackedDeviceIndexInvalid)return;
  appliedX=vx;appliedY=vy;appliedTurnX=tx;appliedTurnY=ty;appliedButtons=buttons;++updates;
  bool ok=true;
  auto scalar=[&](VRInputComponentHandle_t h,float v){ok=(VRDriverInput()->UpdateScalarComponent(h,v,0)==VRInputError_None)&&ok;};
  auto boolean=[&](VRInputComponentHandle_t h,bool v){ok=(VRDriverInput()->UpdateBooleanComponent(h,v,0)==VRInputError_None)&&ok;};
  scalar(x,vx);scalar(y,vy);scalar(turnX,tx);scalar(turnY,ty);
  boolean(touch,std::hypot(vx,vy)>.01f);boolean(click,false);
  boolean(turnTouch,std::hypot(tx,ty)>.01f);boolean(turnClick,false);
  boolean(jump,(buttons&1u)!=0);boolean(crouch,(buttons&2u)!=0);boolean(sprint,(buttons&4u)!=0);updateOk=ok;
  VRServerDriverHost()->TrackedDevicePoseUpdated(index,pose,sizeof pose);
 }
 bool active(){std::lock_guard<std::mutex> lock(mutex);return index!=k_unTrackedDeviceIndexInvalid;}
};

class Provider final:public IServerTrackedDeviceProvider {
 Treadmill treadmill;SOCKET sock=INVALID_SOCKET;bool wsa=false,calibrated=false;
 protocol::PeriodicWorker worker;std::mutex lifecycle;std::atomic<bool> standby{false};std::atomic<unsigned> standby_epoch{0};unsigned seen_standby_epoch=0;
 float baseHeading=0;protocol::Transport transport;Clock::time_point origin=Clock::now();sockaddr_in sender{};
 double seconds()const{return std::chrono::duration<double>(Clock::now()-origin).count();}
public:
 EVRInitError Init(IVRDriverContext*context)override {
  VR_INIT_SERVER_DRIVER_CONTEXT(context);
  WSADATA data{};if(WSAStartup(MAKEWORD(2,2),&data)!=0)return VRInitError_Driver_Failed;wsa=true;
  sock=socket(AF_INET,SOCK_DGRAM,IPPROTO_UDP);
  sockaddr_in addr{};addr.sin_family=AF_INET;addr.sin_port=htons(29741);addr.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
  if(sock==INVALID_SOCKET || bind(sock,reinterpret_cast<sockaddr*>(&addr),sizeof addr)==SOCKET_ERROR){Cleanup();return VRInitError_Driver_Failed;}
  u_long nonblock=1;if(ioctlsocket(sock,FIONBIO,&nonblock)==SOCKET_ERROR){Cleanup();return VRInitError_Driver_Failed;}
  if(!VRServerDriverHost()->TrackedDeviceAdded("arcane-locomotion-001",TrackedDeviceClass_Controller,&treadmill)){Cleanup();return VRInitError_Driver_Failed;}
  standby=false;try {worker.start([this](){this->transport_tick();});} catch(...) {Cleanup();return VRInitError_Driver_Failed;}
  VRDriverLog()->Log("Arcane locomotion: ALOC/ALC2 loopback UDP29741, all inputs expire after 250ms.");return VRInitError_None;
 }
 void Cleanup()override {
  worker.stop();
  treadmill.update();transport.session.active=false;calibrated=false;
  if(sock!=INVALID_SOCKET){closesocket(sock);sock=INVALID_SOCKET;}
  if(wsa){WSACleanup();wsa=false;}
  CleanupDriverContext();
 }
 const char*const*GetInterfaceVersions()override{return k_InterfaceVersions;}
 void RunFrame()override {}
 void transport_tick() {
  std::lock_guard<std::mutex> lock(lifecycle);
  auto& session=transport.session;
  auto epoch=standby_epoch.load(std::memory_order_acquire);
  if(epoch!=seen_standby_epoch || standby.load(std::memory_order_acquire)){seen_standby_epoch=epoch;session.active=false;calibrated=false;treadmill.update();return;}
  double now=seconds();sockaddr_in from{};
  auto receive=[&](char* bytes,int cap,uint16_t& port){int len=sizeof from;int count=recvfrom(sock,bytes,cap,0,reinterpret_cast<sockaddr*>(&from),&len);if(count<=0)return count;if(from.sin_family!=AF_INET || from.sin_addr.s_addr!=htonl(INADDR_LOOPBACK))return 1;port=from.sin_port;return count;};
  if(transport.pump(now,receive,[&](){sender=from;}))calibrated=false;
  TrackedDevicePose_t poses[k_unMaxTrackedDeviceCount]{};
  VRServerDriverHost()->GetRawTrackedDevicePoses(0,poses,k_unMaxTrackedDeviceCount);
  // A slow runtime call must never allow an expired input to be applied or acknowledged.
  if(transport.expire_before_output(seconds()))calibrated=false;
  auto& hmd=poses[k_unTrackedDeviceIndex_Hmd];auto& matrix=hmd.mDeviceToAbsoluteTracking.m;
  bool valid=hmd.bPoseIsValid && hmd.bDeviceIsConnected;
  for(int r=0;r<3;r++)for(int c=0;c<4;c++)valid=valid && std::isfinite(matrix[r][c]);
  bool lostTracking=!valid && (calibrated || (session.active && !session.input.neutral()));
  float height=0,heading=0,pitch=0,roll=0,vx=0,vy=0;
  if(valid){
   height=matrix[1][3];heading=std::atan2(matrix[0][2],matrix[2][2]);
   pitch=std::atan2(-matrix[1][2],std::hypot(matrix[0][2],matrix[2][2]));
   roll=std::atan2(matrix[1][0],matrix[1][1]);
  }
  if(session.active && valid) {
   if(!calibrated){baseHeading=heading;calibrated=true;}
   protocol::travel(session.input,heading-(baseHeading+session.input.chestYaw),vx,vy);
   treadmill.update(vx,vy,session.input.turnX,session.input.turnY,session.input.buttons);
  }else {calibrated=false;treadmill.update();}
  if(session.active){
   uint32_t flags=(treadmill.active()?1u:0u) | (valid?2u:0u) | (calibrated?4u:0u);
   if(!session.input.extended){
    Ack ack{{'A','L','A','K'},session.input.sequence,session.input.generation,flags};
    sendto(sock,reinterpret_cast<const char*>(&ack),sizeof ack,0,reinterpret_cast<sockaddr*>(&sender),sizeof sender);
   }
   AckV2 ack{{'A','K','V','2'},session.input.sequence,session.input.generation,flags,height,heading,pitch,roll};
   sendto(sock,reinterpret_cast<const char*>(&ack),sizeof ack,0,reinterpret_cast<sockaddr*>(&sender),sizeof sender);
  }
  if(lostTracking)session.active=false;
 }
 bool ShouldBlockStandbyMode()override{return false;}
 void EnterStandby()override{std::lock_guard<std::mutex> lock(lifecycle);standby.store(true,std::memory_order_release);standby_epoch.fetch_add(1,std::memory_order_release);transport.session.active=false;calibrated=false;treadmill.update();}
 void LeaveStandby()override{standby.store(false,std::memory_order_release);}
};
static Provider provider;
extern "C" __declspec(dllexport) void*HmdDriverFactory(const char*name,int*error) {
 if(!std::strcmp(name,IServerTrackedDeviceProvider_Version))return &provider;
 if(error)*error=VRInitError_Init_InterfaceNotFound;return nullptr;
}
#endif


