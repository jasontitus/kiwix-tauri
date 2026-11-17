package org.kiwix.mobile

import android.bluetooth.BluetoothGattCharacteristic
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NearbyBluetoothPropertiesTest {
    private val channel = BluetoothGattCharacteristic.PROPERTY_WRITE or BluetoothGattCharacteristic.PROPERTY_INDICATE
    private val extended = BluetoothGattCharacteristic.PROPERTY_EXTENDED_PROPS

    @Test
    fun acceptsAndroidAndObservedCoreBluetoothChannel() {
        assertTrue(NearbyBluetooth.channelPropertiesValid(channel))
        // Physical iPhone 13 exposes 0xA8: write + indicate + extended properties.
        assertTrue(NearbyBluetooth.channelPropertiesValid(0xA8))
    }

    @Test
    fun extendedPropertiesCannotReplaceEitherRequiredOperation() {
        for (properties in listOf(0, extended, BluetoothGattCharacteristic.PROPERTY_WRITE,
            BluetoothGattCharacteristic.PROPERTY_INDICATE,
            extended or BluetoothGattCharacteristic.PROPERTY_WRITE,
            extended or BluetoothGattCharacteristic.PROPERTY_INDICATE)) {
            assertFalse("properties=$properties", NearbyBluetooth.channelPropertiesValid(properties))
        }
    }

    @Test
    fun rejectsOtherAdvertisedOperationModes() {
        for (extra in listOf(BluetoothGattCharacteristic.PROPERTY_READ,
            BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE,
            BluetoothGattCharacteristic.PROPERTY_SIGNED_WRITE,
            BluetoothGattCharacteristic.PROPERTY_BROADCAST, 0x100)) {
            assertFalse("extra=$extra", NearbyBluetooth.channelPropertiesValid(channel or extra))
            assertFalse("extended extra=$extra", NearbyBluetooth.channelPropertiesValid(channel or extended or extra))
        }
    }
}
