# NOTE: à copier sur le raspberry pi et à rouler ensuite. Faut juste insaller python3 avec 'python-can'

# si necessaire:
# $ sudo ip link set can0 up type can bitrate 250000

import can

can.rc['interface'] = 'socketcan'
can.rc['channel'] = 'can0'
can.rc['bitrate'] = 250000

ACTUATOR_COMMAND = 0x00
ACTUATOR_READ_COMMAND = 0x01
MANO_READ_COMMAND = 0x02

with can.Bus() as bus:
    while True:
        command = input("$ ")
        if command.startswith("actuator "):
            args = command.split(" ")
            num = int(args[1])
            if num > 3 or num < 1:
                print("invalid second argument (must be between 1 and 3)")
                continue

            if args[2] == "on":
                bus.send(can.Message(
                         arbitration_id=0x00,
                         data=[ACTUATOR_COMMAND, num, 1],
                         is_extended_id=False
                ))
            elif args[2] == "off":
                bus.send(can.Message(
                         arbitration_id=0x00,
                         data=[ACTUATOR_COMMAND, num, 0],
                         is_extended_id=False
                ))
            else:
                print("Unkown status: " + args[2])

        elif command == "readactuators":
            bus.send(can.Message(
                     arbitration_id=0x00,
                     data=[ACTUATOR_READ_COMMAND],
                     is_extended_id=False
            ))

            msg = bus.recv(timeout=1.0)

            if msg is not None:
                if msg.data[0] == ACTUATOR_READ_COMMAND:
                    for i, state in enumerate(msg.data[1:4]):
                        print(f"Actuator {i + 1} is " + ("off" if state == 0 else ("on" if state == 1 else "moving")))
                else:
                    print("Unknown response")
            else:
                print("No response...")

        elif command == "readmano":
            bus.send(can.Message(
                     arbitration_id=0x00,
                     data=[MANO_READ_COMMAND, 1],
                     is_extended_id=False
            ))

            while True:
                try:
                    msg = bus.recv(timeout=0.5)
                    if msg is None:
                        print("Timeout")
                        break
                    if msg.data[0] != MANO_READ_COMMAND:
                        print("Unknown response")
                        continue

                    mano1 = ((msg.data[1] << 8) + msg.data[2]) / 1000.0
                    mano2 = ((msg.data[3] << 8) + msg.data[4]) / 1000.0

                    print(f"Manometer 1: {mano1}, Manometer 2: {mano2}")
                except KeyboardInterrupt:
                    print("stopping...")
                    break

            bus.send(can.Message(
                     arbitration_id=0x00,
                     data=[MANO_READ_COMMAND, 0],
                     is_extended_id=False
            ))


        else:
            print("unknown command")
