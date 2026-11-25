// Compiled by both the macOS and the iOS runner.
import Foundation

#if os(macOS)
  import AppKit
  import AudioToolbox
  import CoreAudio
  @preconcurrency import FlutterMacOS
#elseif os(iOS)
  import AVFoundation
  import MediaPlayer
  import UIKit
  @preconcurrency import Flutter
#endif

/// The system's output volume, for Rune's volume control while a track plays on a platform host
/// that has no volume of its own.
///
/// Dart reads it with `volume`, whose answer is `{volume, settable}`, and gets `changed` calls
/// with the same map. On macOS Dart sets it with `setVolume`: the default output device's main
/// volume, followed through hardware keys and device changes. A muted output reads as volume 0,
/// and setting a volume unmutes it, as the volume keys do. iOS has no public call to set it:
/// the volume control there is the system's own volume view, in a platform view.
@MainActor
final class SystemVolume {
  static let shared = SystemVolume()

  private var channel: FlutterMethodChannel?
  /// What `changed` last told Dart, so that only a change is reported.
  private var reported: [String: Any]?

  #if os(macOS)
    private var device = AudioObjectID(kAudioObjectUnknown)
    private var volumeListener: AudioObjectPropertyListenerBlock?
    private var defaultDeviceListener: AudioObjectPropertyListenerBlock?
  #elseif os(iOS)
    private var observation: NSKeyValueObservation?
  #endif

  private init() {}

  /// Creates the channel on the engine's messenger and starts following the system volume.
  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(
      name: "ci.not.rune/system_volume", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] (call, result) in
      // Flutter delivers channel calls on the main thread.
      MainActor.assumeIsolated {
        guard let self else {
          result(FlutterMethodNotImplemented)
          return
        }
        self.handle(call, result)
      }
    }
    self.channel = channel
    reported = nil
    startWatching()
  }

  private func handle(_ call: FlutterMethodCall, _ result: @escaping FlutterResult) {
    switch call.method {
    case "volume":
      result(state())
    case "setVolume":
      guard let volume = (call.arguments as? NSNumber)?.doubleValue else {
        result(FlutterError(code: "bad_arguments", message: "A volume is needed.", details: nil))
        return
      }
      if setVolume(min(max(volume, 0), 1)) {
        result(nil)
      } else {
        result(
          FlutterError(
            code: "not_settable", message: "The output's volume cannot be set.", details: nil))
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  /// Tells Dart the volume if it differs from what Dart was told last.
  private func report() {
    let current = state()
    if let reported, NSDictionary(dictionary: reported).isEqual(to: current) { return }
    reported = current
    channel?.invokeMethod("changed", arguments: current)
  }

  #if os(macOS)
    private func state() -> [String: Any] {
      let volume = readVolume()
      let muted = readMuted()
      return [
        "volume": muted ? 0.0 : (volume ?? 1.0),
        "settable": volume != nil && isSettable(Self.volumeAddress()),
      ]
    }

    /// The main volume of the output device as the system's volume keys and menu show it. This is
    /// the AudioHardwareService property `VirtualMainVolume`, not the per-channel
    /// `kAudioDevicePropertyVolumeScalar`: devices with separate channel volumes, or with a
    /// volume only the driver knows, answer for all channels at once, so no channel needs
    /// choosing.
    private static func volumeAddress() -> AudioObjectPropertyAddress {
      AudioObjectPropertyAddress(
        mSelector: kAudioHardwareServiceDeviceProperty_VirtualMainVolume,
        mScope: kAudioDevicePropertyScopeOutput,
        mElement: kAudioObjectPropertyElementMain)
    }

    private static func muteAddress() -> AudioObjectPropertyAddress {
      AudioObjectPropertyAddress(
        mSelector: kAudioDevicePropertyMute,
        mScope: kAudioDevicePropertyScopeOutput,
        mElement: kAudioObjectPropertyElementMain)
    }

    private static func defaultDeviceAddress() -> AudioObjectPropertyAddress {
      AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
    }

    private static func defaultOutputDevice() -> AudioObjectID {
      var address = defaultDeviceAddress()
      var device = AudioObjectID(kAudioObjectUnknown)
      var size = UInt32(MemoryLayout<AudioObjectID>.size)
      let status = AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device)
      return status == noErr ? device : AudioObjectID(kAudioObjectUnknown)
    }

    /// `nil` when the output has no main volume, as with some HDMI and digital outputs.
    private func readVolume() -> Double? {
      guard device != kAudioObjectUnknown else { return nil }
      var address = Self.volumeAddress()
      guard AudioObjectHasProperty(device, &address) else { return nil }
      var volume = Float32(0)
      var size = UInt32(MemoryLayout<Float32>.size)
      let status = AudioObjectGetPropertyData(device, &address, 0, nil, &size, &volume)
      return status == noErr ? Double(volume) : nil
    }

    /// Whether the output is muted; an output without a mute switch never is.
    private func readMuted() -> Bool {
      guard device != kAudioObjectUnknown else { return false }
      var address = Self.muteAddress()
      guard AudioObjectHasProperty(device, &address) else { return false }
      var muted = UInt32(0)
      var size = UInt32(MemoryLayout<UInt32>.size)
      let status = AudioObjectGetPropertyData(device, &address, 0, nil, &size, &muted)
      return status == noErr && muted != 0
    }

    private func isSettable(_ address: AudioObjectPropertyAddress) -> Bool {
      guard device != kAudioObjectUnknown else { return false }
      var address = address
      guard AudioObjectHasProperty(device, &address) else { return false }
      var settable = DarwinBoolean(false)
      let status = AudioObjectIsPropertySettable(device, &address, &settable)
      return status == noErr && settable.boolValue
    }

    private func setVolume(_ volume: Double) -> Bool {
      guard isSettable(Self.volumeAddress()) else { return false }
      // Moving the slider on a muted output unmutes it, like the volume keys.
      if readMuted() && isSettable(Self.muteAddress()) {
        var address = Self.muteAddress()
        var unmuted = UInt32(0)
        AudioObjectSetPropertyData(
          device, &address, 0, nil, UInt32(MemoryLayout<UInt32>.size), &unmuted)
      }
      var address = Self.volumeAddress()
      var value = Float32(volume)
      let status = AudioObjectSetPropertyData(
        device, &address, 0, nil, UInt32(MemoryLayout<Float32>.size), &value)
      return status == noErr
    }

    private func startWatching() {
      // A reattach replaces the listeners rather than adding more.
      stopWatchingDevice()
      if let defaultDeviceListener {
        var address = Self.defaultDeviceAddress()
        AudioObjectRemovePropertyListenerBlock(
          AudioObjectID(kAudioObjectSystemObject), &address, .main, defaultDeviceListener)
      }
      let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
        MainActor.assumeIsolated {
          self?.outputDeviceChanged()
        }
      }
      defaultDeviceListener = listener
      var address = Self.defaultDeviceAddress()
      AudioObjectAddPropertyListenerBlock(
        AudioObjectID(kAudioObjectSystemObject), &address, .main, listener)
      watchDevice(Self.defaultOutputDevice())
    }

    private func stopWatchingDevice() {
      if let volumeListener, device != kAudioObjectUnknown {
        for var address in [Self.volumeAddress(), Self.muteAddress()] {
          AudioObjectRemovePropertyListenerBlock(device, &address, .main, volumeListener)
        }
      }
      volumeListener = nil
      device = AudioObjectID(kAudioObjectUnknown)
    }

    private func watchDevice(_ newDevice: AudioObjectID) {
      device = newDevice
      guard newDevice != kAudioObjectUnknown else { return }
      let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
        MainActor.assumeIsolated {
          self?.report()
        }
      }
      volumeListener = listener
      for var address in [Self.volumeAddress(), Self.muteAddress()] {
        AudioObjectAddPropertyListenerBlock(newDevice, &address, .main, listener)
      }
    }

    /// The default output is another device now, with a volume of its own.
    private func outputDeviceChanged() {
      stopWatchingDevice()
      watchDevice(Self.defaultOutputDevice())
      report()
    }
  #elseif os(iOS)
    private func state() -> [String: Any] {
      // The system's own volume view moves it, so the output always counts as settable here.
      ["volume": Double(AVAudioSession.sharedInstance().outputVolume), "settable": true]
    }

    /// There is no public call to set the system volume; the platform view does that.
    private func setVolume(_ volume: Double) -> Bool { false }

    private func startWatching() {
      // KVO calls back on whatever thread changed the volume, which the documentation does not
      // name, so the handler hops to the main actor rather than assuming it is there.
      observation = AVAudioSession.sharedInstance().observe(\.outputVolume, options: [.new]) {
        @Sendable [weak self] _, _ in
        Task { @MainActor in
          self?.report()
        }
      }
    }

    /// Registers the system's volume view for Dart's `UiKitView`.
    func registerView(with registrar: any FlutterBaseRegistrar) {
      registrar.register(SystemVolumeViewFactory(), withId: "ci.not.rune/system_volume_view")
    }
  #endif
}

#if os(iOS)
  /// Makes the system's volume view: the documented way to let the user change the volume.
  @MainActor
  private final class SystemVolumeViewFactory: NSObject, @preconcurrency FlutterPlatformViewFactory {
    func create(
      withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?
    ) -> FlutterPlatformView {
      SystemVolumePlatformView(frame: frame)
    }
  }

  @MainActor
  private final class SystemVolumePlatformView: NSObject, @preconcurrency FlutterPlatformView {
    private let volumeView: MPVolumeView

    init(frame: CGRect) {
      volumeView = MPVolumeView(frame: frame)
      // `showsRouteButton` is deprecated in favour of AVRoutePickerView, but it is what hides the
      // route button this view still shows. Rune has its own output picker, so the view is only
      // the slider.
      volumeView.showsRouteButton = false
      super.init()
    }

    func view() -> UIView { volumeView }
  }
#endif
